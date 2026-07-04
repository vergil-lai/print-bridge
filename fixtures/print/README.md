# Print Fixtures

Use this folder for manual print validation files:

- `label-60x40.pdf`
- `label-100x150.pdf`
- `label-image.png`
- `label-image.jpg`

The PDF page size should match the label size being tested. Do not use an A4 PDF with a label placed inside it for label printer validation.

## macOS Manual Validation

Current local validation results:

- `lpstat -p -d`: failed with `lpstat: Bad file descriptor`; this machine has no usable default print destination.
- `pnpm tauri dev`: started successfully and compiled/running locally.
- `curl -s http://127.0.0.1:17890/health`: failed to connect inside the sandbox; succeeded outside the sandbox with `{"status":"ok","service":"print-bridge"}`.

Real physical printing was not validated in this run because this machine has no usable default print destination.
