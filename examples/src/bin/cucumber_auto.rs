/// Usage: Set the environment variable CUCUMBER_DEPLOYER_COMPOSE to use the
/// Compose deployer. Otherwise, the Local deployer is used by default.
///
/// Example using docker compose deployer:
/// ```sh
/// CUCUMBER_DEPLOYER_COMPOSE=1 cargo run -p runner-examples --bin cucumber_auto -- --name "Run auto deployer smoke scenario"
/// ```
/// Example using local deployer:
/// ```sh
/// cargo run -p runner-examples --bin cucumber_auto --  --name "Run auto deployer smoke scenario"
/// ```
use std::{fs, io};

use cucumber::{World, WriterExt, writer, writer::Verbosity};
use cucumber_ext::{DeployerKind, TestingFrameworkWorld};
use runner_examples::defaults::{init_logging_defaults, init_node_log_dir_defaults, init_tracing};

#[tokio::main]
async fn main() {
    println!("args: {:?}", std::env::args());

    let deployer = if std::env::var("CUCUMBER_DEPLOYER_COMPOSE").ok().is_some() {
        DeployerKind::Compose
    } else {
        DeployerKind::Local
    };
    println!("Running with '{:?}'", deployer);

    init_logging_defaults();
    init_node_log_dir_defaults(deployer);
    init_tracing();

    // Print current directory for debugging
    if let Ok(current_dir) = std::env::current_dir() {
        println!("Current directory: {:?}", current_dir);
    }

    let file = fs::File::create("cucumber-output-junit.xml").unwrap();
    let world = TestingFrameworkWorld::cucumber()
        .repeat_failed()
        // following config needed to use eprint statements in the tests
        .max_concurrent_scenarios(1)
        .fail_on_skipped()
        .fail_fast()
        .with_writer(
            writer::Summarize::new(writer::Basic::new(
                io::stdout(),
                writer::Coloring::Auto,
                Verbosity::ShowWorldAndDocString,
            ))
            .tee::<TestingFrameworkWorld, _>(writer::JUnit::for_tee(file, 0))
            .normalized(),
        )
        .before(move |feature, _rule, scenario, world| {
            Box::pin(async move {
                println!(
                    "\nStarting '{}' : '{}' : '{}'\n",
                    feature.name, scenario.keyword, scenario.name
                ); // This will be printed into the stdout_buffer
                if let Err(e) = world.set_deployer(deployer) {
                    panic!("Failed to set deployer: {}", e);
                }
            })
        });
    world.run_and_exit("examples/cucumber/features/").await;
}
