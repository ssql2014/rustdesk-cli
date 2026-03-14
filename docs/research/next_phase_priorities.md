# Next Phase Priorities: AI Inference & Protocol Robustness

This document outlines the strategic priorities for the `rustdesk-cli` project following the successful deployment of the RMSNorm operator.

## 1. Essential Operators for Transformer Inference

Beyond RMSNorm, a functional inference pipeline for modern Large Language Models (LLMs) like Llama 3 or Mistral requires the following operators.

| Operator | Description | Priority |
| :--- | :--- | :--- |
| **MatMul (Matrix Multiplication)** | The computational core of Linear layers and Attention projections. Requires optimized BLAS or hardware-specific kernels. | **Critical** |
| **SiLU (Sigmoid Linear Unit)** | The primary activation function used in Llama-style architectures ($x \cdot \text{sigmoid}(x)$). | **High** |
| **Embedding** | Maps input token IDs to their corresponding high-dimensional vector representations. | **High** |
| **SoftMax** | Normalizes attention scores into a probability distribution. Critical for the Attention mechanism. | **High** |
| **Multi-Head Attention** | The core logic that performs scaled dot-product attention across multiple heads. | **High** |
| **RoPE (Rotary Position Encoding)** | Encodes positional information by rotating components of the query and key vectors in complex space. | **Medium** |

## 2. Protocol Feature Roadmap

To transition from a "working prototype" to a "production-ready agent tool," the following RustDesk protocol features must be prioritized in `rustdesk-cli`.

### Daemon Stability & Persistence
- **Issue:** Background connections currently drop during transient network blips.
- **Need:** Implement the standard RustDesk **Heartbeat** mechanism and a robust **Reconnection State Machine** that can resume a session (using the `service_id` for terminal persistence) without local manual intervention.

### File Transfer (Model Weights)
- **Issue:** Inference requires moving large `.safetensors` or `.gguf` files (often 5GB+).
- **Need:** Implement the `FileAction` and `FileResponse` protobuf flow. This allows the CLI to "push" model weights to the remote peer's `/home/evas` directory efficiently using chunked transfers.

### Exec Timeout & Streaming
- **Issue:** Complex inference tasks can exceed the default 30s timeout.
- **Need:** Support custom command deadlines and real-time STDOUT/STDERR streaming during `exec` commands so the agent can monitor progress of long-running computations.

## 3. Recommended Next 3 Tasks

1.  **Task 1: Implement Heartbeat & Auto-Reconnection.**
    Add a background heartbeat task to `src/daemon.rs` and update the connection logic to automatically retry the Phase 1-4 handshake if the socket is lost.
2.  **Task 2: Basic File Transfer (Push).**
    Implement the `FileAction::SendRequest` and chunked data block protocol to allow the agent to deploy model weights to the remote node.
3.  **Task 3: MLP Block Operators (MatMul + SiLU).**
    Expand the remote Python operator kit to support Matrix Multiplication and SiLU activations, enabling the execution of a full MLP (Feed-Forward) layer.
