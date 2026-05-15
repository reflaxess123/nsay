r"""
Convert RealBasicVSR pretrained .pth weights to a TorchScript .pt file
that the future nsay-vidsr-* sidecars can load via tch-rs (PLAN.md F4).

Why TorchScript, not ONNX:
  RealBasicVSR's BasicVSR backbone uses operators that don't survive a
  clean ONNX export to ORT 2.x — see codex audit + open-mmlab/mmagic#1004.
  TorchScript via torch.jit.trace bakes the recurrent graph and ships
  through tch-rs without those round-trip headaches.

Setup (one-time, separate Python env recommended):
    python -m venv .venv-vsr
    .\.venv-vsr\Scripts\Activate.ps1
    pip install torch==2.5.0 mmcv-full mmagic numpy pillow

Run:
    # download RealBasicVSR_x4.pth from the official release first:
    #   https://github.com/ckkelvinchan/RealBasicVSR/releases
    python scripts/convert-realbasicvsr.py \\
        --pth path/to/RealBasicVSR_x4.pth \\
        --out src-tauri/models/realbasicvsr-x4.pt \\
        --window 15 --width 320 --height 180

Args:
    --pth     path to the official .pth checkpoint
    --out     where to write the TorchScript .pt
    --window  number of frames the traced graph expects per call (RealBasicVSR
              processes a clip; the typical inference window is 15)
    --width   trace-time input width  (model is fully convolutional so any
              size works at runtime, but tracing fixes the inference shape
              of the SPyNet flow estimator)
    --height  trace-time input height

Output is a .pt file the Rust sidecar will load with `tch::CModule::load`.

Status:
    Stub — needs the live mmagic / mmcv install on the dev box plus the
    RealBasicVSR weights. Filling in the actual `torch.jit.trace` call is
    F4.3; this file is committed so the workflow is documented and the
    PowerShell side can call into a known location.
"""

import argparse
import sys
from pathlib import Path


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--pth",    type=Path, required=True)
    p.add_argument("--out",    type=Path, required=True)
    p.add_argument("--window", type=int, default=15)
    p.add_argument("--width",  type=int, default=320)
    p.add_argument("--height", type=int, default=180)
    args = p.parse_args()

    if not args.pth.exists():
        print(f"error: --pth not found: {args.pth}", file=sys.stderr)
        return 1

    try:
        import torch
        from mmagic.models.editors.real_basicvsr.real_basicvsr_net import RealBasicVSRNet  # type: ignore
    except ImportError as e:
        print(
            "error: missing dependency. Install with:\n"
            "  pip install torch==2.5.0 mmcv-full mmagic\n"
            f"original error: {e}",
            file=sys.stderr,
        )
        return 2

    print(f">> loading {args.pth}")
    state = torch.load(args.pth, map_location="cpu", weights_only=False)
    if isinstance(state, dict) and "state_dict" in state:
        state = state["state_dict"]
    # Strip "generator." prefix that mmagic checkpoints carry on top.
    state = {k.replace("generator.", "", 1): v for k, v in state.items()}

    model = RealBasicVSRNet().eval()
    missing, unexpected = model.load_state_dict(state, strict=False)
    if missing:    print(f"   missing keys:    {len(missing)}")
    if unexpected: print(f"   unexpected keys: {len(unexpected)}")

    dummy = torch.randn(1, args.window, 3, args.height, args.width)
    print(f">> tracing with window={args.window} hxw={args.height}x{args.width}")
    with torch.no_grad():
        traced = torch.jit.trace(model, dummy)

    args.out.parent.mkdir(parents=True, exist_ok=True)
    traced.save(str(args.out))
    print(f">> wrote {args.out} ({args.out.stat().st_size // (1024 * 1024)} MB)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
