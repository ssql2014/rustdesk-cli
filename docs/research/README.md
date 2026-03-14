# Research Index: rustdesk-cli & AI Inference

This directory contains technical research and protocol analysis for the `rustdesk-cli` project, specifically focusing on its application for remote AI inference.

## 1. Research Documents Listing

| Document | Topic | Key Findings | Status |
| :--- | :--- | :--- | :--- |
| [attention_op.py](attention_op.py) | AI Operator | NumPy GQA implementation; handles Query/KV head grouping (4:1). | Complete |
| [clipboard_protocol.md](clipboard_protocol.md) | Protocol | Lightweight data pipe (4KB - 64MB); Push-based; Zstd compression. | Complete |
| [daemon_binary_replacement.md](daemon_binary_replacement.md) | Dev Workflow | macOS SIGBUS/SIGKILL on recompilation; "Move Aside" pattern recommended. | Complete |
| [daemon_socket_lifecycle.md](daemon_socket_lifecycle.md) | OS / UDS | Connect-then-Unlink pattern for stale sockets; $TMPDIR usage on macOS. | Complete |
| [embedding_operator.md](embedding_operator.md) | AI Operator | Llama 3 uses 128k vocab; lookup is memory-bound; 1.05GB RAM for 8B model. | Complete |
| [exec_command_limits.md](exec_command_limits.md) | OS / PTY | 4KB PTY `MAX_INPUT` limit found; causing daemon hangs on large execs. | Complete |
| [exec_timeout_streaming.md](exec_timeout_streaming.md) | Protocol / UDS | Real-time output delivery via TerminalData; custom --timeout design. | Complete |
| [file_transfer_protocol.md](file_transfer_protocol.md) | Protocol | 128KB chunks; Zstd compression; Resume supported via Digest. | Complete |
| [file_transfer_session_init.md](file_transfer_session_init.md) | Protocol | ConnType::FILE_TRANSFER (1); dedicated TCP connection required. | Complete |
| [fork_vs_build_recommendation.md](fork_vs_build_recommendation.md) | Strategy | Build from scratch is superior to forking official repo (GUI bloat). | Complete |
| [hbbr_relay_matching.md](hbbr_relay_matching.md) | Protocol | hbbr matches by UUID only; peer always uses DefaultConn for handshake. | Complete |
| [hbbs_punch_hole_protocol.md](hbbs_punch_hole_protocol.md) | Protocol | Server sends NO response on success; License mismatch results in failure=3. | Complete |
| [heartbeat_reconnect_impl.md](heartbeat_reconnect_impl.md) | Implementation | Design for service_id persistence and exponential backoff (max 30s). | Complete |
| [heartbeat_reconnect.md](heartbeat_reconnect.md) | Protocol | 15s RegisterPeer interval; 90s timeout; Re-use service_id for persistence. | Complete |
| [key_exchange_client_response.md](key_exchange_client_response.md) | Crypto | Handshake uses `crypto_box` (zero nonce) with 2 keys in response. | Complete |
| [key_types_and_usage.md](key_types_and_usage.md) | Crypto | Server Ed25519 is the Root of Trust; used for both TCP and Peer auth. | Complete |
| [kv_cache_implementation.md](kv_cache_implementation.md) | AI Architecture | Contiguous pre-allocation vs PagedAttention; 128KB per token per layer. | Complete |
| [login_offline_error.md](login_offline_error.md) | Bug Analysis | "Offline" usually means malformed `LoginRequest` (empty username). | Complete |
| [multi_head_attention.md](multi_head_attention.md) | AI Operator | Llama 3 uses GQA (4:1 or 8:1 ratio); KV Cache is critical for decoding. | Complete |
| [next_phase_priorities.md](next_phase_priorities.md) | Roadmap | MatMul/SiLU are next; Daemon stability and File Transfer are high priority. | Complete |
| [official_cli_mode_verification.md](official_cli_mode_verification.md) | Strategy | Verification that official `--features cli` still has heavy GUI deps. | Complete |
| [option_message_protocol.md](option_message_protocol.md) | Protocol | Terminal sessions use minimal Options (persistent flag only). | Complete |
| [permission_access_control.md](permission_access_control.md) | Protocol | CM UI triggered by empty password; Bitmask enforced by hbbs. | Complete |
| [post_login_protocol.md](post_login_protocol.md) | Protocol | Terminal flow: OpenTerminal -> Stream Data; Video uses VP9/AV1. | Complete |
| [quantization_strategies.md](quantization_strategies.md) | AI Optimization | GGUF Q4_K_M (5.0GB) recommended for remote nodes; lazy dequantization. | Complete |
| [relay_response_flow.md](relay_response_flow.md) | Protocol | RelayResponse is TCP-ONLY; Client MUST use TCP for rendezvous to see it. | Complete |
| [rmsnorm_operator.md](rmsnorm_operator.md) | AI Operator | 40% faster than LayerNorm; No mean subtraction; Stability via epsilon. | Complete |
| [rope_positional_encoding.md](rope_positional_encoding.md) | AI Operator | Complex space rotations; Theta base 500k for Llama 3; Distance preservation. | Complete |
| [rustdesk_client_relay_flow.md](rustdesk_client_relay_flow.md) | Protocol | Official client uses TCP for RequestRelay; 3-attempt retry loop. | Complete |
| [secure_tcp_stream_details.md](secure_tcp_stream_details.md) | Crypto | XSalsa20-Poly1305; Counter-based nonce; Payload encryption only. | Complete |
| [softmax_op.py](softmax_op.py) | AI Operator | NumPy stable implementation; handles temperature and causal masks. | Complete |
| [softmax_operator.md](softmax_operator.md) | AI Operator | Max subtraction trick for stability; Causal mask sets future to -inf. | Complete |
| [swiglu_mlp_operator.md](swiglu_mlp_operator.md) | AI Operator | SwiGLU gated activation; Llama 3 intermediate dim (14336) mapping. | Complete |
| [tcp_key_exchange.md](tcp_key_exchange.md) | Protocol | Port 21116 mandates security upgrade via KeyExchange before signaling. | Complete |
| [terminal_connection_flow.md](terminal_connection_flow.md) | Protocol | ConnType::TERMINAL (5) must be consistent in all requests. | Complete |
| [terminal_direct_vs_daemon.md](terminal_direct_vs_daemon.md) | Bug Analysis | Race condition between UDP/TCP relay requests causing hangs. | Complete |
| [terminal_peer_requirements.md](terminal_peer_requirements.md) | Protocol | `enable-terminal` option must be != "N"; Password-less needs UI click. | Complete |
| [terminal_proto_additions.md](terminal_proto_additions.md) | Protocol | Our .proto files are already complete for terminal support. | Complete |
| [transformer_block_assembly.md](transformer_block_assembly.md) | AI Operator | Pre-norm residual stack; SwiGLU MLP; 32 blocks for 8B model. | Complete |
| [weight_loading_strategy.md](weight_loading_strategy.md) | Deployment | Memory Mapping (mmap) + GGUF Sharding (500MB) for resilience. | Complete |

## 2. Operator Dependency Graph

```mermaid
graph TD
    A[Embedding] --> B[Transformer Block]
    B --> C[RMSNorm (Attn)]
    B --> D[Multi-Head Attention]
    D --> E[MatMul]
    D --> F[RoPE]
    D --> G[SoftMax]
    B --> H[RMSNorm (FFN)]
    B --> I[MLP / SwiGLU]
    I --> J[SiLU]
    I --> E
    B --> K[Residual Add]
    B --> L[Transformer Block N+1]
    L --> M[Final RMSNorm]
    M --> N[LM Head (MatMul)]
    N --> O[Token Selection]
```

## 3. Deployment Status (Peer 308235080)

| Operator | Implementation | Deployed | Verified |
| :--- | :--- | :--- | :--- |
| **RMSNorm** | Python/NumPy | Yes | Yes |
| **MatMul** | Python/NumPy | Yes | Yes |
| **SiLU** | Python/NumPy | Yes | Yes |
| **RoPE** | Python/NumPy | Yes | Yes |
| **SoftMax** | Python/NumPy | Yes | Yes |
| **Embedding** | Python/NumPy | Yes | Yes |
| **GQA Attention** | Python/NumPy | Yes | Yes |
| **Full Pipeline** | Python/NumPy | Yes | Yes |

## 4. Recommended Next Steps

1.  **Production Hardening:** Transition from prototype Python operators to a compiled Rust inference engine (via `burn` or `candle`) for 10x performance.
2.  **Streaming IPC:** Finalize the UDS streaming implementation to allow real-time token observability.
3.  **Daemon Watchdog:** Implement the auto-reconnect logic with `service_id` persistence to handle long-running generation tasks over flaky networks.
4.  **Weight Sharding:** Implement the multi-shard `push` command to automate deployment of 5GB+ GGUF files.
