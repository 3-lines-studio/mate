use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use chrono::{DateTime, Datelike, Local, TimeZone, Timelike};

use super::resolve::resolve_client;
use super::{Deps, Notifier};
use crate::agent::AgentSession;
use crate::config::{ModelConfig, ProviderConfig, ScheduledJob};
use crate::provider::Client;
use crate::tools::Registry;
use std::collections::HashMap;

use crate::agent::SubagentDef;
use tokio::sync::Mutex as TokioMutex;

const JOB_TIMEOUT: Duration = Duration::from_secs(180);
const NOTIFY_TIMEOUT: Duration = Duration::from_secs(30);

struct JobCtx {
    idx: usize,
    job: ScheduledJob,
    store: crate::session::store::Store,
    default_client: Client,
    registry: Arc<Registry>,
    system_prompt: String,
    cwd: String,
    subagents: HashMap<String, SubagentDef>,
    models: Vec<ModelConfig>,
    providers: Vec<ProviderConfig>,
}

pub fn start_scheduler(deps: &Deps, notifier: Arc<dyn Notifier + Send + Sync>) {
    if deps.config.schedule.jobs.is_empty() {
        return;
    }

    log::info!(
        "scheduler jobs registered, jobs={}",
        deps.config.schedule.jobs.len()
    );

    let jobs = deps.config.schedule.jobs.clone();
    let store = deps.store.clone();
    let default_client = deps.client.clone();
    let registry = deps.registry.clone();
    let system_prompt = deps.system_prompt.clone();
    let cwd = deps.cwd.clone();
    let subagents = deps.subagents.clone();
    let models = deps.config.models.clone();
    let providers = deps.config.providers.clone();

    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                log::error!("scheduler runtime: {}", e);
                return;
            }
        };

        rt.block_on(async move {
            for (idx, job) in jobs.into_iter().enumerate() {
                if job.cron.is_empty() {
                    continue;
                }
                let ctx = JobCtx {
                    idx,
                    job,
                    store: store.clone(),
                    default_client: default_client.clone(),
                    registry: registry.clone(),
                    system_prompt: system_prompt.clone(),
                    cwd: cwd.clone(),
                    subagents: subagents.clone(),
                    models: models.clone(),
                    providers: providers.clone(),
                };
                let running = Arc::new(AtomicBool::new(false));
                tokio::spawn(schedule_loop(ctx, notifier.clone(), running));
            }
            std::future::pending::<()>().await;
        });
    });
}

async fn schedule_loop(
    ctx: JobCtx,
    notifier: Arc<dyn Notifier + Send + Sync>,
    running: Arc<AtomicBool>,
) {
    let spec = match parse_cron(&ctx.job.cron) {
        Ok(s) => s,
        Err(e) => {
            log::error!(
                "schedule cron parse idx={} cron={} err={}",
                ctx.idx,
                ctx.job.cron,
                e
            );
            return;
        }
    };

    loop {
        let now = Local::now();
        let fire = match next_fire(&spec, &now) {
            Some(t) => t,
            None => {
                sleep(Duration::from_secs(3600)).await;
                continue;
            }
        };
        let delay_ms = (fire - now).num_milliseconds().max(0) as u64;
        sleep(Duration::from_millis(delay_ms)).await;

        if running.swap(true, Ordering::SeqCst) {
            log::debug!("schedule job skipped (still running) idx={}", ctx.idx);
            continue;
        }

        match tokio::time::timeout(JOB_TIMEOUT, run_job(&ctx)).await {
            Ok(Ok(result)) => {
                log::info!(
                    "schedule job completed idx={} output_len={}",
                    ctx.idx,
                    result.len()
                );
                notify(&notifier, &ctx.job.channel, &result).await;
            }
            Ok(Err(e)) => {
                log::error!("schedule job error idx={} err={}", ctx.idx, e);
                notify(
                    &notifier,
                    &ctx.job.channel,
                    &format!("Scheduled job error: {}", e),
                )
                .await;
            }
            Err(_) => log::warn!("schedule job timed out idx={} timeout=180s", ctx.idx),
        }

        running.store(false, Ordering::SeqCst);
    }
}

async fn run_job(ctx: &JobCtx) -> Result<String, String> {
    log::info!(
        "schedule job triggered idx={} cron={} channel={}",
        ctx.idx,
        ctx.job.cron,
        ctx.job.channel
    );

    let store_arc = Arc::new(TokioMutex::new(ctx.store.clone()));
    let sess = {
        let mut s = store_arc.lock().await;
        s.create().map_err(|e| e.to_string())?
    };
    let sess_id = sess.id.clone();

    let client = if !ctx.job.model.is_empty() {
        let (c, _) = resolve_client(&ctx.job.model, &ctx.models, &ctx.providers, false)?;
        c
    } else {
        ctx.default_client.clone()
    };

    let mut asession = AgentSession::new(
        store_arc.clone(),
        sess,
        Arc::new(client),
        ctx.registry.clone(),
        ctx.system_prompt.clone(),
        ctx.cwd.clone(),
    );
    asession.set_subagents(ctx.subagents.clone());

    let mut events = asession.prompt(&ctx.job.prompt);
    let mut sb = String::new();
    while let Some(ev) = events.recv().await {
        if let crate::agent::EventKind::TextDelta(delta) = ev.kind {
            sb.push_str(&delta);
        }
    }

    {
        let mut s = store_arc.lock().await;
        let _ = s.delete(&sess_id);
    }

    if sb.is_empty() {
        log::warn!("schedule job produced no text output idx={}", ctx.idx);
        Ok("(no output)".to_string())
    } else {
        Ok(sb)
    }
}

async fn notify(notifier: &Arc<dyn Notifier + Send + Sync>, channel: &str, message: &str) {
    if channel.is_empty() {
        return;
    }
    let n = notifier.clone();
    let ch = channel.to_string();
    let msg = message.to_string();
    let _ = tokio::time::timeout(
        NOTIFY_TIMEOUT,
        tokio::task::spawn_blocking(move || {
            if let Err(e) = n.schedule_notify(&ch, &msg) {
                log::error!("schedule notify failed channel={} err={}", ch, e);
            }
        }),
    )
    .await;
}

fn sleep(d: Duration) -> tokio::time::Sleep {
    tokio::time::sleep(d)
}

struct CronSpec {
    minute: Vec<u32>,
    hour: Vec<u32>,
    dom: Vec<u32>,
    month: Vec<u32>,
    dow: Vec<u32>,
    dom_star: bool,
    dow_star: bool,
}

fn parse_field(field: &str, lo: u32, hi: u32) -> Result<Vec<u32>, String> {
    if field == "*" || field == "?" {
        return Ok((lo..=hi).collect());
    }
    let mut out: Vec<u32> = Vec::new();
    for term in field.split(',') {
        let term = term.trim();
        if term.is_empty() {
            return Err("empty cron term".to_string());
        }
        let (range_part, step) = match term.find('/') {
            Some(i) => {
                let step: u32 = term[i + 1..]
                    .parse()
                    .map_err(|_| format!("bad step in {}", term))?;
                if step == 0 {
                    return Err(format!("zero step in {}", term));
                }
                (&term[..i], step)
            }
            None => (term, 1),
        };
        let (start, end) = if range_part == "*" {
            (lo, hi)
        } else if let Some(i) = range_part.find('-') {
            let s: u32 = range_part[..i]
                .parse()
                .map_err(|_| format!("bad range start in {}", term))?;
            let e: u32 = range_part[i + 1..]
                .parse()
                .map_err(|_| format!("bad range end in {}", term))?;
            (s, e)
        } else {
            let v: u32 = range_part
                .parse()
                .map_err(|_| format!("bad value in {}", term))?;
            if step != 1 { (v, hi) } else { (v, v) }
        };
        if start > end {
            return Err(format!("descending range in {}", term));
        }
        let mut v = start;
        while v <= end {
            if v >= lo && v <= hi {
                out.push(v);
            }
            let next = match v.checked_add(step) {
                Some(n) => n,
                None => break,
            };
            if next <= v {
                break;
            }
            v = next;
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

fn parse_cron(expr: &str) -> Result<CronSpec, String> {
    let fields: Vec<&str> = expr.split_whitespace().collect();
    if fields.len() != 5 {
        return Err(format!(
            "expected 5 cron fields, got {}: {}",
            fields.len(),
            expr
        ));
    }
    let dom_star = fields[2] == "*" || fields[2] == "?";
    let dow_star = fields[4] == "*" || fields[4] == "?";
    let minute = parse_field(fields[0], 0, 59)?;
    let hour = parse_field(fields[1], 0, 23)?;
    let dom = parse_field(fields[2], 1, 31)?;
    let month = parse_field(fields[3], 1, 12)?;
    let mut dow = parse_field(fields[4], 0, 7)?;
    for v in &mut dow {
        if *v == 7 {
            *v = 0;
        }
    }
    dow.sort();
    dow.dedup();
    Ok(CronSpec {
        minute,
        hour,
        dom,
        month,
        dow,
        dom_star,
        dow_star,
    })
}

fn cron_matches<Tz: TimeZone>(spec: &CronSpec, dt: &DateTime<Tz>) -> bool {
    if !spec.minute.contains(&dt.minute()) {
        return false;
    }
    if !spec.hour.contains(&dt.hour()) {
        return false;
    }
    if !spec.month.contains(&dt.month()) {
        return false;
    }
    let weekday = (dt.weekday() as u32 + 1) % 7;
    let dom_ok = spec.dom.contains(&dt.day());
    let dow_ok = spec.dow.contains(&weekday);
    match (spec.dom_star, spec.dow_star) {
        (true, true) => true,
        (false, true) => dom_ok,
        (true, false) => dow_ok,
        (false, false) => dom_ok || dow_ok,
    }
}

fn next_fire<Tz: TimeZone>(spec: &CronSpec, after: &DateTime<Tz>) -> Option<DateTime<Tz>> {
    let mut t = after
        .clone()
        .with_second(0)
        .unwrap()
        .with_nanosecond(0)
        .unwrap()
        + chrono::Duration::minutes(1);
    let cap = after.clone() + chrono::Duration::days(366 * 8);
    while t <= cap {
        if cron_matches(spec, &t) {
            return Some(t);
        }
        t += chrono::Duration::minutes(1);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_field_every_15() {
        let v = parse_field("*/15", 0, 59).unwrap();
        assert_eq!(v, vec![0, 15, 30, 45]);
    }

    #[test]
    fn test_parse_field_range() {
        let v = parse_field("1-5", 0, 59).unwrap();
        assert_eq!(v, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn test_parse_field_list() {
        let v = parse_field("1,3,5", 0, 59).unwrap();
        assert_eq!(v, vec![1, 3, 5]);
    }

    #[test]
    fn test_parse_field_step_from_value() {
        let v = parse_field("5/15", 0, 59).unwrap();
        assert_eq!(v, vec![5, 20, 35, 50]);
    }

    #[test]
    fn test_parse_field_range_step() {
        let v = parse_field("1-10/3", 0, 59).unwrap();
        assert_eq!(v, vec![1, 4, 7, 10]);
    }

    #[test]
    fn test_parse_dow_sunday7() {
        let spec = parse_cron("0 0 * * 7").unwrap();
        assert!(spec.dow.contains(&0));
        assert!(!spec.dow.contains(&7));
    }

    #[test]
    fn test_parse_cron_fields() {
        let spec = parse_cron("0 9 * * 1-5").unwrap();
        assert_eq!(spec.minute, vec![0]);
        assert_eq!(spec.hour, vec![9]);
        assert!(spec.dom_star);
        assert!(!spec.dow_star);
        assert_eq!(spec.dow, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn test_parse_cron_bad_field_count() {
        assert!(parse_cron("0 9 * *").is_err());
        assert!(parse_cron("0 9 * * 1 2").is_err());
    }

    #[test]
    fn test_next_fire_daily_9am() {
        let spec = parse_cron("0 9 * * *").unwrap();
        let base = Local.with_ymd_and_hms(2024, 1, 15, 10, 30, 0).unwrap();
        let nf = next_fire(&spec, &base).unwrap();
        assert_eq!(nf.hour(), 9);
        assert_eq!(nf.minute(), 0);
        assert_eq!(nf.day(), 16);
    }

    #[test]
    fn test_next_fire_already_due_keeps_field() {
        let spec = parse_cron("30 9 * * *").unwrap();
        let base = Local.with_ymd_and_hms(2024, 1, 15, 9, 0, 0).unwrap();
        let nf = next_fire(&spec, &base).unwrap();
        assert_eq!(nf.hour(), 9);
        assert_eq!(nf.minute(), 30);
        assert_eq!(nf.day(), 15);
    }

    #[test]
    fn test_next_fire_weekdays() {
        let spec = parse_cron("0 9 * * 1-5").unwrap();
        let base = Local.with_ymd_and_hms(2024, 1, 12, 10, 0, 0).unwrap();
        let nf = next_fire(&spec, &base).unwrap();
        assert_eq!(nf.hour(), 9);
        let wd = (nf.weekday() as u32 + 1) % 7;
        assert!((1..=5).contains(&wd), "expected weekday, got {}", wd);
    }

    #[test]
    fn test_dom_dow_or_semantics() {
        let spec = parse_cron("0 0 15 * 1").unwrap();
        let base = Local.with_ymd_and_hms(2024, 1, 15, 0, 1, 0).unwrap();
        let nf = next_fire(&spec, &base).unwrap();
        let wd = (nf.weekday() as u32 + 1) % 7;
        assert!(nf.day() == 15 || wd == 1, "OR semantics failed");
        assert!(nf.day() != 15, "expected a Monday, not the 15th");
    }

    #[test]
    fn test_dom_only_when_dow_star() {
        let spec = parse_cron("0 0 15 * *").unwrap();
        let base = Local.with_ymd_and_hms(2024, 1, 10, 0, 0, 0).unwrap();
        let nf = next_fire(&spec, &base).unwrap();
        assert_eq!(nf.day(), 15);
    }
}
