# Research: NAT Traversal and Relay Performance

This document investigates the RustDesk NAT traversal mechanism, the performance characteristics of relayed vs. direct connections, and the strategy for optimizing throughput in `rustdesk-cli`.

## 1. NAT Hole-Punching Mechanism

RustDesk implements NAT traversal primarily through UDP hole punching, brokered by the rendezvous server (`hbbs`).

### Step-by-Step Flow:
1. **Detection:** The client and peer detect their NAT types (Asymmetric/Cone vs. Symmetric) by querying `hbbs`.
2. **Request:** The client sends a `PunchHoleRequest` to `hbbs`.
3. **Coordination:** `hbbs` forwards a `PunchHole` message to the target peer via its active registration socket.
4. **Punching:** Both parties attempt to send UDP packets to each other's public IP and port (the "hole punch").
5. **Success:** If a packet gets through, the parties exchange encrypted handshakes directly over UDP.
6. **Fallback:** If no direct connection is established within a timeout, the client sends a `RequestRelay` to `hbbs` to initiate a session via `hbbr`.

### Protobuf Traces:
- **`PunchHoleRequest`**: Includes `nat_type`, `udp_port`, and `force_relay`.
- **`PunchHole`**: The server-forwarded version of the request.
- **`RequestRelay`**: Explicitly requests `hbbs` to broker a meeting point on `hbbr`.

## 2. Performance: Direct P2P vs. Relay

| Metric | Direct P2P | Relay (hbbr) |
| :--- | :--- | :--- |
| **Latency** | Minimum (Physical distance) | High (Extra hop through hbbr) |
| **Throughput** | Limited by physical link | Limited by hbbr bandwidth & CPU |
| **Stability** | Depends on NAT mapping persistence | Highly stable |
| **Cost** | Free (no server load) | High (consumes server bandwidth) |

### Evas Relay Estimates (115.238.185.55:50077):
The current relay server is a shared resource.
- **Estimated Throughput:** 10-50 Mbps per session (depending on global load).
- **Inference Impact:** Pushing 4.65GB weights via relay takes ~15-60 minutes. Direct P2P on a gigabit LAN/WAN could reduce this to < 5 minutes.

## 3. Support for Direct P2P

### NAT Compatibility Matrix:
| Client \ Peer | Open/Public | Asymmetric | Symmetric |
| :--- | :--- | :--- | :--- |
| **Open/Public** | **Direct** | **Direct** | **Direct** |
| **Asymmetric** | **Direct** | **Direct** | Relay (Hard) |
| **Symmetric** | **Direct** | Relay (Hard) | **Relay** |

*Note: "Hard" means hole punching fails 90% of the time, though some UPnP/NAT-PMP tricks can sometimes force it.*

## 4. NAT Type Detection Logic

The official client (`src/common.rs:test_nat_type`) performs a "Dual Port Test":
1. Connect to `hbbs:21116`.
2. Connect to `hbbs:21115` (or any other port on the same IP).
3. If the external port reported by the server is the same for both connections, the NAT is **Asymmetric**.
4. If the ports differ, it is **Symmetric**.

## 5. Proposed Design: `--prefer-direct`

Currently, `rustdesk-cli` defaults to `force_relay = true` for simplicity. To support high-throughput transfers, we should implement a tiered connection strategy.

### Tiered Logic:
1. **Attempt Direct:** Send `PunchHoleRequest` with `force_relay = false`.
2. **Short Timeout:** Wait 2.0 seconds for a direct UDP handshake.
3. **Relay Fallback:** If no `SignedId` is received over UDP, immediately initiate Phase 3 (Relay) via TCP `RequestRelay`.

### CLI Implementation:
```bash
# Default behavior (safe, uses relay)
rustdesk-cli push model.gguf

# High-performance behavior (attempts P2P first)
rustdesk-cli push model.gguf --prefer-direct
```

## Conclusion

Direct P2P is critical for large weight deployments. By implementing NAT type detection and a tiered fallback strategy, we can significantly improve the performance of the `rustdesk-cli` inference pipeline while reducing the load on the `hbbr` relay servers.
