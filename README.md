# Turbofuro
Visual programming language for building cool things in record time.

This repository contains the worker application, runtime, Turbofuro Expression Language (TEL) and its bindings. You can find the editor and more details about this project on the [Turbofuro website](https://turbofuro.com).

*Note: This project is experimental and under active development. It is not recommended for production use yet.*

## Getting started
Download a latest release for your platform from the [GitHub releases page](https://github.com/turbofuro/turbofuro/releases) or install the binary using `cargo install turbofuro_worker`. There is also an official [Docker image](https://hub.docker.com/r/turbofuro/worker) you can use.

Once you have the binary you can run it with the following command:
```bash
turbofuro_worker --token <YOUR_MACHINE_TOKEN>
```
You can get your machine token from the machine details on [Turbofuro](https://turbofuro.com).

### Local development
This project contains a Cargo workspace with multiple crates. To build it locally, you need to install Rust and Cargo. You can do that by following the instructions on the [Rust website](https://www.rust-lang.org/tools/install).

After that, you can clone the repository and build the project with the following command:
```bash
cargo build --release
```
Once the build is completed you can use the `turbofuro_worker` binary in the `target/release` folder.

The turbofuro_worker project includes many examples of modules. You can run the test configuration with the following command:
```bash
cd turbofuro_worker
cargo run -- --config test_config.json
```

#### Running with tokio-console
The worker uses [Tokio](https://tokio.rs/) as a async runtime. You can run and connect to the worker with [tokio-console](https://github.com/tokio-rs/console) to inspect running tasks and resources.
```
RUSTFLAGS="--cfg tokio_unstable" cargo run -- --token XXX
```

## Contributing
We welcome all contributions with 💛 

Feel free to create issues including those with feature suggestions. If you want to help, but not sure how, reach out to [@pr0gramista](https://github.com/pr0gramista) (Twitter/LinkedIn/email) directly.

## License
Turbofuro Worker is licensed under [Apache-2.0](https://opensource.org/license/apache-2-0/).

Happy Coding! 🚀
