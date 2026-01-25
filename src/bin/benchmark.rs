//! Headless benchmark tool for CHOUIBOT
//!
//! Simulates chat messages and user joins to measure CPU usage without GUI/TUI.
//! Run with: cargo run --bin benchmark

use std::time::{Duration, Instant};
use tokio::sync::broadcast;

use choui_the_no_gui_chatbot::state::AppEvent;

/// Read CPU usage from /proc/stat (Linux only)
fn get_cpu_usage() -> Option<f64> {
    let content = std::fs::read_to_string("/proc/stat").ok()?;
    let first_line = content.lines().next()?;
    let parts: Vec<&str> = first_line.split_whitespace().collect();

    if parts.len() < 5 || parts[0] != "cpu" {
        return None;
    }

    let user: u64 = parts[1].parse().ok()?;
    let nice: u64 = parts[2].parse().ok()?;
    let system: u64 = parts[3].parse().ok()?;
    let idle: u64 = parts[4].parse().ok()?;

    let total = user + nice + system + idle;
    let used = user + nice + system;

    Some((used as f64 / total as f64) * 100.0)
}

struct CpuMonitor {
    samples: Vec<f64>,
    start_time: Instant,
}

impl CpuMonitor {
    fn new() -> Self {
        Self {
            samples: Vec::new(),
            start_time: Instant::now(),
        }
    }

    fn sample(&mut self) {
        if let Some(usage) = get_cpu_usage() {
            self.samples.push(usage);
        }
    }

    fn report(&self) {
        if self.samples.is_empty() {
            println!("No CPU samples collected.");
            return;
        }

        let avg = self.samples.iter().sum::<f64>() / self.samples.len() as f64;
        let max = self.samples.iter().cloned().fold(0.0_f64, f64::max);
        let min = self.samples.iter().cloned().fold(f64::MAX, f64::min);

        println!("\n=== CPU Usage Report ===");
        println!("Duration: {:.2}s", self.start_time.elapsed().as_secs_f64());
        println!("Samples: {}", self.samples.len());
        println!("Average CPU: {:.2}%", avg);
        println!("Min CPU: {:.2}%", min);
        println!("Max CPU: {:.2}%", max);
    }
}

#[tokio::main]
async fn main() {
    println!("=== CHOUIBOT Headless Benchmark ===\n");

    // Create broadcast channel (like the real app)
    let (tx, mut rx) = broadcast::channel::<AppEvent>(100);

    // CPU Monitor
    let mut cpu_monitor = CpuMonitor::new();

    // Configuration
    let num_messages = 50;
    let num_joins = 10;
    let message_delay_ms = 100;

    println!("Configuration:");
    println!("  Messages to send: {}", num_messages);
    println!("  Joins to simulate: {}", num_joins);
    println!("  Delay between events: {}ms", message_delay_ms);
    println!();

    // Spawn event processor (simulates what the main app does)
    let processor_tx = tx.clone();
    let processor_handle = tokio::spawn(async move {
        let mut rx = processor_tx.subscribe();
        let mut message_count = 0;
        let mut join_count = 0;

        loop {
            match rx.recv().await {
                Ok(event) => {
                    match event {
                        AppEvent::ChatMessage { user, text } => {
                            message_count += 1;
                            // Simulate processing (like AI would do)
                            // NOTE: We're NOT calling AI here to avoid network calls
                            let _ = format!("Processing message from {}: {}", user, text);
                        }
                        AppEvent::UserJoined(user) => {
                            join_count += 1;
                            let _ = format!("User {} joined", user);
                        }
                        AppEvent::Info(msg) => {
                            if msg == "BENCHMARK_DONE" {
                                println!(
                                    "\nProcessed {} messages, {} joins",
                                    message_count, join_count
                                );
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
            }
        }
    });

    // Start benchmark
    println!("Starting benchmark...\n");
    let start = Instant::now();

    // Sample CPU before
    cpu_monitor.sample();

    // Simulate events
    for i in 0..num_messages {
        let event = if i % (num_messages / num_joins) == 0 && i > 0 {
            // Simulate a join every N messages
            AppEvent::UserJoined(format!("TestUser{}", i / (num_messages / num_joins)))
        } else {
            AppEvent::ChatMessage {
                user: format!("User{}", i % 5),
                text: format!("Test message number {} with some content to process", i),
            }
        };

        let _ = tx.send(event);

        // Sample CPU periodically
        if i % 10 == 0 {
            cpu_monitor.sample();
        }

        tokio::time::sleep(Duration::from_millis(message_delay_ms as u64)).await;
    }

    // Signal done
    let _ = tx.send(AppEvent::Info("BENCHMARK_DONE".to_string()));

    // Wait for processor
    let _ = processor_handle.await;

    // Final CPU sample
    cpu_monitor.sample();

    let elapsed = start.elapsed();
    println!("\nBenchmark completed in {:.2}s", elapsed.as_secs_f64());

    // Report CPU usage
    cpu_monitor.report();

    // Calculate throughput
    let events_per_second = (num_messages as f64) / elapsed.as_secs_f64();
    println!("\nThroughput: {:.2} events/second", events_per_second);
}
