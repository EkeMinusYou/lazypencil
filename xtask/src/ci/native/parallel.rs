//! Join every bounded CI lane before returning success or failure.

use anyhow::{Result, ensure};

pub type Task<'a> = (&'static str, Box<dyn FnOnce() -> Result<()> + Send + 'a>);

pub fn run(tasks: Vec<Task<'_>>) -> Result<()> {
    ensure!(
        (1..=3).contains(&tasks.len()),
        "Expected one to three CI lanes"
    );
    std::thread::scope(|scope| {
        let handles = tasks
            .into_iter()
            .map(|(name, task)| (name, scope.spawn(task)))
            .collect::<Vec<_>>();
        let mut failures = Vec::new();
        for (name, handle) in handles {
            match handle.join() {
                Ok(Ok(())) => {}
                Ok(Err(error)) => failures.push(format!("{name}: {error:#}")),
                Err(_) => failures.push(format!("{name}: task panicked")),
            }
        }
        ensure!(
            failures.is_empty(),
            "Parallel CI failed:\n{}",
            failures.join("\n")
        );
        Ok(())
    })
}
