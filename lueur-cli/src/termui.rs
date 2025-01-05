use std::collections::HashMap;
use std::time::Duration;

use colored::Colorize;
use indicatif::MultiProgress;
use indicatif::ProgressBar;
use indicatif::ProgressStyle;
use tokio::sync::broadcast;
use tokio::sync::broadcast::Receiver;

use crate::event::FaultEvent;
use crate::event::TaskId;
use crate::event::TaskProgressEvent;

/// Struct to hold information about each task
struct TaskInfo {
    pb: ProgressBar,
    url: String,
    fault: Option<FaultEvent>,
    status_code: Option<u16>,
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
                            TaskProgressEvent::Started { id, ts, url } => {
                                let pb = multi.add(ProgressBar::new_spinner());
                                pb.set_style(style.clone());
                                pb.enable_steady_tick(Duration::from_millis(80));

                                let c = "URL:".dimmed();
                                let m = format!("{} {}", c, url.bright_blue());

                                pb.set_message(m);

                                let task_info =
                                    TaskInfo { pb, url, fault: None, status_code: None };
                                task_map.insert(id, task_info);
                            }
                            TaskProgressEvent::IpResolved { id, ts, domain, time_taken } => {},
                            TaskProgressEvent::WithFault { id, ts, fault } => {
                                if let Some(task_info) = task_map.get_mut(&id) {
                                    task_info.fault = Some(fault.clone());

                                    let c = "URL:".dimmed();
                                    let u = format!("{} {}", c, task_info.url.bright_blue());

                                    let f = fault_to_string(&task_info.fault);

                                    task_info.pb.set_message(format!("{} | {} | ...", u, f));
                                }
                            }
                            TaskProgressEvent::FaultApplied { id, ts, fault, direction } => {}
                            TaskProgressEvent::FaultComputed { id, ts, fault, direction } => {}
                            TaskProgressEvent::ResponseReceived { id, ts, status_code } => {
                                if let Some(task_info) = task_map.get_mut(&id) {
                                    let c = "Status:".dimmed();
                                    let mut m = "".clear();

                                    m = if (200..300).contains(&status_code) {
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

                                    let c = "URL:".dimmed();
                                    let u = format!("{} {}", c, task_info.url.bright_blue());

                                    let c = "Fault:".dimmed();
                                    let f = fault_to_string(&task_info.fault);

                                    task_info.pb.set_message(format!("{} | {} | {}", u, f, s));
                                }
                            }
                            TaskProgressEvent::Completed {
                                id,
                                ts,
                                time_taken,
                                from_downstream_length,
                                from_upstream_length,
                            } => {
                                if let Some(task_info) = task_map.remove(&id) {
                                    let c = "Status:".dimmed();
                                    let mut m = "".white();
                                    let status_code = task_info.status_code.unwrap_or(0);

                                    m = if (200..300).contains(&status_code) {
                                        status_code.to_string().green()
                                    } else if (400..500).contains(&status_code) {
                                        status_code.to_string().yellow()
                                    } else if status_code == 0 {
                                        "-".to_string().dimmed()
                                    } else {
                                        status_code.to_string().red()
                                    };

                                    let s = format!("{} {}", c, m);

                                    let c = "URL:".dimmed();
                                    let u = format!("{} {}", c, task_info.url.bright_blue());

                                    let f = fault_to_string(&task_info.fault);

                                    let c = "Duration:".dimmed();
                                    let o = "Sent/Received:".dimmed();
                                    let d = format!(
                                        "{} {:.2}ms | {} ⭫{}/b ⭭{}/b",
                                        c,
                                        time_taken.as_millis_f64(),
                                        o,
                                        from_downstream_length,
                                        from_upstream_length
                                    );

                                    task_info.pb.finish_with_message(format!(
                                        "{} | {} | {} | {} |",
                                        u, f, s, d
                                    ));
                                }
                            }
                            TaskProgressEvent::Error { id, ts, error } => {
                                if let Some(task_info) = task_map.remove(&id) {
                                    let c = "Status:".dimmed();
                                    let mut m = "".white();
                                    let status_code = task_info.status_code.unwrap();

                                    m = if (200..300).contains(&status_code) {
                                        status_code.to_string().green()
                                    } else if (400..500).contains(&status_code) {
                                        status_code.to_string().yellow()
                                    } else if status_code == 0 {
                                        "-".to_string().dimmed()
                                    } else {
                                        status_code.to_string().red()
                                    };

                                    let s = format!("{} {}", c, m);

                                    let c = "URL:".dimmed();
                                    let u = format!("{} {}", c, task_info.url.bright_blue());

                                    let f = fault_to_string(&task_info.fault);

                                    let c = "Error:".dimmed();
                                    let e = format!("{} {}", c, error);

                                    task_info
                                        .pb
                                        .set_message(format!("{} | {} | {} | {}", u, f, s, e));
                                }
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        break;
                    }
                    Err(broadcast::error::RecvError::Lagged(count)) => {
                        eprintln!("Missed {} messages", count);
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
            FaultEvent::Latency { delay } => {
                let c = "Fault:".dimmed();
                let f = "latency".yellow();
                format!("{} {} {}ms", c, f, delay.as_millis_f64())
            }
            FaultEvent::Dns { triggered } => {
                let c = "Fault:".dimmed();
                let f = "dns".yellow();
                format!(
                    "{} {} {}",
                    c,
                    f,
                    if *triggered {
                        "triggered".to_string()
                    } else {
                        "not triggered".to_string()
                    }
                )
            }
            FaultEvent::Bandwidth { bps } => todo!(),
            FaultEvent::Jitter { amplitude, frequency } => todo!(),
            FaultEvent::PacketLoss { loss_probability } => todo!(),
        },
        None => "".to_string(),
    }
}
