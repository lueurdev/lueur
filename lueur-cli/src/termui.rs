use std::collections::HashMap;
use std::time::Duration;

use colored::*;
use indicatif::MultiProgress;
use indicatif::ProgressBar;
use indicatif::ProgressStyle;
use tokio::sync::broadcast;
use tokio::sync::broadcast::Receiver;

use crate::event::FaultEvent;
use crate::event::TaskId;
use crate::event::TaskProgressEvent;
use crate::types::Direction;

/// Struct to hold information about each task
struct TaskInfo {
    pb: ProgressBar,
    url: String,
    resolution_time: f64,
    fault: Option<FaultEvent>,
    status_code: Option<u16>,
    events: Vec<FaultEvent>,
}

/// Handles displayable events and updates the progress bars accordingly
pub async fn handle_displayable_events(
    mut receiver: Receiver<TaskProgressEvent>,
) {
    let multi = MultiProgress::new();

    let style = ProgressStyle::default_bar()
        .template("{spinner:.green} {msg}")
        .unwrap()
        .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]) // Cus
        .progress_chars("=> ");

    let mut task_map: HashMap<TaskId, TaskInfo> = HashMap::new();

    loop {
        tokio::select! {
            event = receiver.recv() => {
                match event {
                    Ok(event) => {
                        match event {
                            TaskProgressEvent::Started { id, ts: _, url } => {
                                let pb = multi.add(ProgressBar::new_spinner());
                                pb.set_style(style.clone());
                                pb.enable_steady_tick(Duration::from_millis(80));

                                let m = format!("{} {}", "URL:".dimmed(), url.bright_blue());

                                pb.set_message(m);

                                let task_info =
                                    TaskInfo { pb, url, resolution_time: 0.0, fault: None, status_code: None, events: Vec::new() };
                                task_map.insert(id, task_info);
                            }
                            TaskProgressEvent::IpResolved { id, ts: _, domain: _, time_taken } => {
                                if let Some(task_info) = task_map.get_mut(&id) {
                                    task_info.resolution_time = time_taken;

                                    let u = format!("{} {}", "URL:".dimmed(), task_info.url.bright_blue());
                                    let d = format!("{} {}ms", "DNS:".dimmed(), time_taken);

                                    task_info.pb.set_message(format!("{} | {} | ...", u, d));
                                }
                            },
                            TaskProgressEvent::WithFault { id, ts: _, fault, direction: _ } => {
                                if let Some(task_info) = task_map.get_mut(&id) {
                                    task_info.fault = Some(fault.clone());

                                    let u = format!("{} {}", "URL:".dimmed(), task_info.url.bright_blue());
                                    let d = format!("{} {}ms", "DNS:".dimmed(), task_info.resolution_time);
                                    let f = fault_to_string(&task_info.fault);

                                    task_info.pb.set_message(format!("{} | {} | {} | ...", u, d, f));
                                }
                            }
                            TaskProgressEvent::FaultApplied { id, ts: _, fault, direction } => {
                                if let Some(task_info) = task_map.get_mut(&id) {

                                    if let FaultEvent::Latency { delay } = &fault {
                                        task_info.events.push(fault.clone());
                                        if let Some(latency) = delay {
                                            let max_events = 20;

                                            let sparkline: String = task_info.events.iter()
                                                .filter_map(|f| {
                                                    if let FaultEvent::Latency { delay: Some(d) } = f {
                                                        Some(latency_to_sparkline_char(*d))
                                                    } else {
                                                        None
                                                    }
                                                })
                                                .rev()
                                                .take(max_events)
                                                .collect::<Vec<_>>()
                                                .iter()
                                                .rev()
                                                .map(|c| c.to_string())
                                                .collect();

                                            let u = format!("{} {}", "URL:".dimmed(), task_info.url.bright_blue());
                                            let d = format!("{} {}ms", "DNS:".dimmed(), task_info.resolution_time);
                                            let f = fault_to_string(&task_info.fault);

                                            task_info.pb.set_message(format!("{} | {} | {} | {} | ...", u, d, f, sparkline));
                                        }
                                    } else
                                    if let FaultEvent::Bandwidth { bps } = &fault {
                                        task_info.events.push(fault.clone());
                                        if let Some(rate) = bps {
                                            let formatted_rate = format!("{}{}", direction_character(direction), format_bandwidth(*rate));

                                            let u = format!("{} {}", "URL:".dimmed(), task_info.url.bright_blue());
                                            let d = format!("{} {}ms", "DNS:".dimmed(), task_info.resolution_time);
                                            let f = fault_to_string(&task_info.fault);

                                            task_info.pb.set_message(format!("{} | {} | {} | {} | ...", u, d, f, formatted_rate));
                                        }
                                    } else
                                    if let FaultEvent::PacketLoss { } = &fault {
                                        task_info.events.push(fault.clone());
                                        let formatted_rate = "".to_string();

                                        let u = format!("{} {}", "URL:".dimmed(), task_info.url.bright_blue());
                                        let d = format!("{} {}ms", "DNS:".dimmed(), task_info.resolution_time);
                                        let f = fault_to_string(&task_info.fault);

                                        task_info.pb.set_message(format!("{} | {} | {} | {} | ...", u, d, f, formatted_rate));
                                    }
                                }
                            }
                            TaskProgressEvent::ResponseReceived { id, ts: _, status_code } => {
                                if let Some(task_info) = task_map.get_mut(&id) {
                                    let c = "Status:".dimmed();
                                    let m = if (200..300).contains(&status_code) {
                                        status_code.to_string().green()
                                    } else if (400..500).contains(&status_code) {
                                        status_code.to_string().yellow()
                                    } else if status_code == 0 {
                                        "-".to_string().dimmed()
                                    } else {
                                        status_code.to_string().red()
                                    };

                                    let s = format!("{} {}", c, m);

                                    task_info.status_code = Some(status_code);
                                    let u = format!("{} {}", "URL:".dimmed(), task_info.url.bright_blue());
                                    let d = format!("{} {}ms", "DNS:".dimmed(), task_info.resolution_time);
                                    let f = fault_to_string(&task_info.fault);

                                    task_info.pb.set_message(format!("{} | {} | {} | {}", u, d, f, s));
                                }
                            }
                            TaskProgressEvent::Completed {
                                id,
                                ts: _,
                                time_taken,
                                from_downstream_length,
                                from_upstream_length,
                            } => {
                                if let Some(task_info) = task_map.remove(&id) {
                                    let c = "Status:".dimmed();
                                    let status_code = task_info.status_code.unwrap_or(0);

                                    let m = if (200..300).contains(&status_code) {
                                        status_code.to_string().green()
                                    } else if (400..500).contains(&status_code) {
                                        status_code.to_string().yellow()
                                    } else if status_code == 0 {
                                        "-".to_string().dimmed()
                                    } else {
                                        status_code.to_string().red()
                                    };

                                    let s = format!("{} {}", c, m);
                                    let u = format!("{} {}", "URL:".dimmed(), task_info.url.bright_blue());
                                    let d = format!("{} {}ms", "DNS:".dimmed(), task_info.resolution_time);

                                    let mut t: String = task_info.events.iter()
                                        .filter_map(|f| {
                                            if let FaultEvent::Latency { delay: Some(d) } = f {
                                                Some(latency_to_sparkline_char(*d))
                                            } else {
                                                None
                                            }
                                        })
                                        .rev()
                                        .take(20)
                                        .collect::<Vec<_>>()
                                        .iter()
                                        .rev()
                                        .map(|c| c.to_string())
                                        .collect();

                                    if let Some(fault) = &task_info.fault {
                                        if let FaultEvent::Bandwidth { bps: _ } = &fault {
                                            let v: Vec<usize> = task_info.events.iter()
                                                .map(|f| {
                                                    if let FaultEvent::Bandwidth { bps: Some(d) } = f { *d } else { 0 }
                                                })
                                                .collect();

                                            let sum = v.iter().sum::<usize>() as usize;
                                            let count = v.len();
                                            if count > 0 {
                                                t = format!("~{}", format_bandwidth(sum/count as usize));
                                            }
                                        } else
                                        if let FaultEvent::Bandwidth { bps } = &fault {
                                            if let Some(rate) = bps {
                                                let formatted_rate = format!("{}" , format_bandwidth(*rate));

                                                let u = format!("{} {}", "URL:".dimmed(), task_info.url.bright_blue());
                                                let d = format!("{} {}ms", "DNS:".dimmed(), task_info.resolution_time);
                                                let f = fault_to_string(&task_info.fault);

                                                task_info.pb.set_message(format!("{} | {} | {} | {} | ...", u, d, f, formatted_rate));
                                            }
                                        } else
                                        if let FaultEvent::PacketLoss { } = &fault {
                                            let formatted_rate = "".to_string();

                                            let u = format!("{} {}", "URL:".dimmed(), task_info.url.bright_blue());
                                            let d = format!("{} {}ms", "DNS:".dimmed(), task_info.resolution_time);
                                            let f = fault_to_string(&task_info.fault);

                                            task_info.pb.set_message(format!("{} | {} | {} | {} | ...", u, d, f, formatted_rate));
                                        }
                                    }

                                    let f = fault_to_string(&task_info.fault);
                                    let h = format!(
                                        "{} {:.2}ms | {} ⭫{}/b ⭭{}/b",
                                        "Duration:".dimmed(),
                                        time_taken.as_millis_f64(),
                                        "Sent/Received:".dimmed(),
                                        from_downstream_length,
                                        from_upstream_length
                                    );

                                    task_info.pb.finish_with_message(format!(
                                        "{} | {} | {} | {} | {} | {} |",
                                        u, d, f, t, s, h
                                    ));
                                }
                            }
                            TaskProgressEvent::Error { id, ts: _, error } => {
                                if let Some(task_info) = task_map.remove(&id) {
                                    let u = format!("{} {}", "URL:".dimmed(), task_info.url.bright_blue());
                                    let d = format!("{} {:.2}ms", "DNS:".dimmed(), task_info.resolution_time);
                                    let f = fault_to_string(&task_info.fault);
                                    let e = format!("{} {}", "Failed:".red(), error);

                                    task_info
                                        .pb
                                        .finish_with_message(format!("{} | {} | {} | {} |", u, d, f, e));
                                }
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        break;
                    }
                    Err(broadcast::error::RecvError::Lagged(count)) => {
                        tracing::warn!("Missed {} events that couldn't be part of the output", count);
                    }
                }
            }
        }
    }

    // Clear the MultiProgress once all tasks are done
    multi.clear().unwrap();
}

fn fault_to_string(fault: &Option<FaultEvent>) -> String {
    match fault {
        Some(fault) => match fault {
            FaultEvent::Latency { delay: _ } => {
                let c = "Fault:".dimmed();
                let f = "latency".yellow();
                format!("{} {}", c, f)
            }
            FaultEvent::Dns { triggered } => {
                let c = "Fault:".dimmed();
                let f = "dns".yellow();
                format!(
                    "{} {} {}",
                    c,
                    f,
                    if triggered.unwrap() {
                        "triggered".to_string()
                    } else {
                        "not triggered".to_string()
                    }
                )
            }
            FaultEvent::Bandwidth { bps: _ } => {
                let c = "Fault:".dimmed();
                let f = "bandwidth".yellow();
                format!("{} {}", c, f)
            }
            FaultEvent::Jitter { amplitude: _, frequency: _ } => {
                let c = "Fault:".dimmed();
                let f = "jitter".yellow();
                format!("{} {}", c, f)
            }
            FaultEvent::PacketLoss {} => {
                let c = "Fault:".dimmed();
                let f = "packet loss".yellow();
                format!("{} {}", c, f)
            }
        },
        None => "".to_string(),
    }
}

/// Maps a latency duration to a colored Unicode character for the sparkline.
/// The mapping is based on latency thresholds in milliseconds.
fn latency_to_sparkline_char(latency: Duration) -> ColoredString {
    let millis = latency.as_secs_f64() * 1000.0;

    if millis < 50.0 {
        '▁'.to_string().green()
    } else if millis < 100.0 {
        '▂'.to_string().green()
    } else if millis < 200.0 {
        '▃'.to_string().yellow()
    } else if millis < 400.0 {
        '▄'.to_string().yellow()
    } else if millis < 800.0 {
        '▅'.to_string().red()
    } else {
        '▆'.to_string().red()
    }
}

/// Helper function to format bandwidth rate into human-readable string.
fn format_bandwidth(bps: usize) -> String {
    if bps >= 1_000_000_000 {
        format!("{:.2} GBps", bps as f64 / 1_000_000_000.0)
    } else if bps >= 1_000_000 {
        format!("{:.2} MBps", bps as f64 / 1_000_000.0)
    } else if bps >= 1_000 {
        format!("{:.2} KBps", bps as f64 / 1_000.0)
    } else {
        format!("{} Bps", bps)
    }
}

fn direction_character(direction: Direction) -> String {
    match direction {
        Direction::Ingress => "⭭".to_string(),
        Direction::Egress => "⭫".to_string(),
        Direction::Both => "⭭⭫".to_string(),
    }
}
