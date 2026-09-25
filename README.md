# routerd

`routerd` is a lightweight, high-performance intelligent model router and protocol gateway daemon built for `syntropd`. It provides multi-provider LLM routing, difficulty-tier evaluation, kernel PSI/telemetry integration, systemd socket activation, and pure Rust Varlink IPC.

## Architecture

- **`crates/routerd-core`**: Core routing engine, multi-dimensional scoring formula (latency, cost, context limits, difficulty tiers `fast`/`hard`), multi-provider protocol adapters (OpenAI-compatible, MiniMax, remote LAN Ollama, local syntrop Varlink bridge), kernel PSI telemetry client, and zero cross-chat context contamination.
- **`crates/routerd-daemon`**: Axum/Hyper dual-stack reverse proxy (TCP port 32768 and `/run/syntrop/router.sock`), pure Rust `$LISTEN_FDS` socket activation, pure Rust Varlink IPC server (`io.syntrop.Router1`), `sd_notify` heartbeat, and `/proc/self/statm` RSS memory monitor (<15MB RSS target).
- **`crates/routerctl`**: CLI control tool for inspecting router status, testing providers, benchmarking TTFT/latency, querying available models, and testing routing decisions.
- **`systemd/`**: Socket and service unit definitions (`routerd.socket`, `routerd.service`) and default configuration template (`routerd.toml`).

## License

Apache-2.0
