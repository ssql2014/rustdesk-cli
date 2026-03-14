# Deployment Plan: RMSNorm Operator

This document outlines the steps to deploy and verify the RMSNorm operator on the remote Linux server (Peer ID: `308235080`) using `rustdesk-cli exec`.

## 1. Python Implementation (NumPy)

We will use a pure Python + NumPy implementation to avoid heavy framework dependencies on the remote node.

```python
import numpy as np

def rms_norm(x, gamma, eps=1e-6):
    """
    RMSNorm implementation using NumPy.
    x: Input tensor of shape (..., d)
    gamma: Learnable scaling parameter of shape (d,)
    eps: Numerical stability constant
    """
    # 1. Square: x^2
    # 2. Mean: mean(x^2) along last dimension
    ms = np.mean(np.square(x), axis=-1, keepdims=True)
    
    # 3. Reciprocal Square Root: 1 / sqrt(ms + eps)
    # 4. Scale: x * rsqrt
    x_normed = x * (1.0 / np.sqrt(ms + eps))
    
    # 5. Apply Gain: x_normed * gamma
    return x_normed * gamma
```

## 2. Correctness Test Script

The following script validates that the implementation matches the expected behavior (unit variance before scaling).

```python
import numpy as np

def test_rmsnorm():
    d = 128
    x = np.random.randn(10, d).astype(np.float32)
    gamma = np.ones(d).astype(np.float32)
    eps = 1e-6
    
    out = rms_norm(x, gamma, eps)
    
    # Verify shape
    assert out.shape == x.shape, f"Shape mismatch: {out.shape} vs {x.shape}"
    
    # Verify RMS property (RMS of normed output should be ~1.0)
    # RMS = sqrt(mean(out^2))
    rms = np.sqrt(np.mean(np.square(out), axis=-1))
    np.testing.assert_allclose(rms, 1.0, atol=1e-3)
    
    print("RMSNorm verification successful.")

if __name__ == "__main__":
    test_rmsnorm()
```

## 3. Deployment Commands

Execute these commands via `rustdesk-cli exec` from the local machine.

### Step 3.1: Create implementation and test file
```bash
rustdesk-cli exec --command "cat <<'EOF' > /home/evas/rmsnorm_op.py
import numpy as np

def rms_norm(x, gamma, eps=1e-6):
    ms = np.mean(np.square(x), axis=-1, keepdims=True)
    return x * (1.0 / np.sqrt(ms + eps)) * gamma

def test_rmsnorm():
    d = 128
    x = np.random.randn(10, d).astype(np.float32)
    gamma = np.ones(d).astype(np.float32)
    out = rms_norm(x, gamma)
    rms = np.sqrt(np.mean(np.square(out), axis=-1))
    np.testing.assert_allclose(rms, 1.0, atol=1e-3)
    print('SUCCESS: RMSNorm verification passed.')

if __name__ == '__main__':
    test_rmsnorm()
EOF"
```

### Step 3.2: Run verification
```bash
rustdesk-cli exec --command "python3 /home/evas/rmsnorm_op.py"
```

## 4. Expected Output Format

On successful execution, the command should return:
```text
SUCCESS: RMSNorm verification passed.
```

If it fails, it will raise an `AssertionError` or `ModuleNotFoundError` (if numpy is missing).
