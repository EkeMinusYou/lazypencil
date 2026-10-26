use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    about = "Development, CI, distribution and verification tasks",
    version
)]
struct Arguments {
    #[command(subcommand)]
    command: Task,
}

#[derive(Subcommand)]
enum Task {
    Test(lazypencil_xtask::dev::Arguments),
    Evidence(lazypencil_xtask::evidence::Arguments),
    #[command(subcommand)]
    Icons(lazypencil_xtask::icons::Command),
    Shader(lazypencil_xtask::shader::Arguments),
    #[command(subcommand)]
    Fixtures(lazypencil_xtask::fixtures::Command),
    Measure(lazypencil_xtask::performance::Arguments),
    MeasureTui(lazypencil_xtask::performance::tui::Arguments),
    ProfileNative(lazypencil_xtask::performance::profile::Arguments),
    Terminal(lazypencil_xtask::terminal::Arguments),
    #[command(subcommand)]
    Notices(lazypencil_xtask::notices::Command),
    #[command(subcommand)]
    Ci(lazypencil_xtask::ci::Command),
    #[command(subcommand)]
    Dist(lazypencil_xtask::dist::Command),
}

fn main() -> Result<()> {
    match Arguments::parse().command {
        Task::Test(arguments) => arguments.run(),
        Task::Evidence(arguments) => arguments.run(),
        Task::Icons(command) => command.run(),
        Task::Shader(arguments) => arguments.run(),
        Task::Fixtures(command) => command.run(),
        Task::Measure(arguments) => arguments.run(),
        Task::MeasureTui(arguments) => arguments.run(),
        Task::ProfileNative(arguments) => arguments.run(),
        Task::Terminal(arguments) => arguments.run(),
        Task::Notices(command) => command.run(),
        Task::Ci(command) => command.run(),
        Task::Dist(command) => command.run(),
    }
}
