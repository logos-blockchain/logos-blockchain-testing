use runner_examples::cucumber::{
    Mode, init_logging_defaults, init_node_log_dir_defaults, init_tracing, run,
};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    init_logging_defaults();
    init_node_log_dir_defaults(Mode::Host);
    init_tracing();

    run(Mode::Host).await;
}
