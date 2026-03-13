# RustDesk Test Configuration

## Self-Hosted Server

| Setting | Value |
|---------|-------|
| ID Server | 115.238.185.55:50076 |
| Relay Server | 115.238.185.55:50077 |
| API Server | http://115.238.185.55:50074 |
| Key | SWc0NIWF0wR7kd8rHdGNaCHXtp7dirUImEtrVmRfQdc= |

## Test Target

| Setting | Value |
|---------|-------|
| Machine ID | 308235080 |
| Password | Evas@2026 |
| OS | Ubuntu |

## Usage

```bash
# Connect to test machine
rustdesk-cli connect 308235080 --password Evas@2026 \
  --id-server 115.238.185.55:50076 \
  --relay-server 115.238.185.55:50077 \
  --key SWc0NIWF0wR7kd8rHdGNaCHXtp7dirUImEtrVmRfQdc=
```
