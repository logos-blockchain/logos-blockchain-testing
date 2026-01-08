use std::{fs, io};

use cucumber::{World, WriterExt, writer, writer::Verbosity};
use cucumber_ext::{DeployerKind, TestingFrameworkWorld};
use runner_examples::defaults::{init_logging_defaults, init_node_log_dir_defaults, init_tracing};

#[tokio::main]
async fn main() {
    println!(
        "CUCUMBER_DEPLOYER_KIND: {:?}",
        std::env::var("CUCUMBER_DEPLOYER_KIND")
    );
    println!("args: {:?}", std::env::args());

    // Check if deployer is already set in the environment
    let deployer_env = std::env::var("CUCUMBER_DEPLOYER_KIND").ok();
    let mut deployer = deployer_env.clone().map(|m| match m.as_str() {
        "compose" => DeployerKind::Compose,
        _ => DeployerKind::Local,
    });

    let mut filtered_args = Vec::new();
    for arg in std::env::args() {
        if arg == "--deployer=compose" {
            deployer = Some(DeployerKind::Compose);
        } else if arg == "--deployer=local" {
            deployer = Some(DeployerKind::Local);
        } else {
            filtered_args.push(arg);
        }
    }

    // If deployer was set by arg, re-exec with env var set and without the deployer arg
    if deployer_env.is_none() && filtered_args.len() != std::env::args().count() {
        let mut cmd = std::process::Command::new(&filtered_args[0]);
        cmd.args(&filtered_args[1..]);
        cmd.env(
            "CUCUMBER_DEPLOYER_KIND",
            match deployer.unwrap() {
                DeployerKind::Compose => "compose",
                DeployerKind::Local => "local",
            },
        );
        let status = cmd.status().expect("Failed to re-exec");
        std::process::exit(status.code().unwrap_or(1));
    }

    let deployer = deployer.unwrap_or_else(|| {
        println!(
            "'--deployer' not specified, defaulting to `--deployer=local` (specify with '--deployer=compose' or '--deployer=local')"
        );
        DeployerKind::Local
    });

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
