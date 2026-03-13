a# WPT workflow

This project now includes a lightweight WPT runner integrated into `stokes-browser`.

## 1) Bootstrap WPT checkout

```powershell
just wpt-bootstrap
```

This clones (or updates) `third_party/wpt` and rebuilds its manifest.

## 2) Start WPT server (separate terminal)

```powershell
just wpt-serve
```

The Rust runner expects WPT to be reachable at `http://127.0.0.1:8000` by default.

## 3) Run the smoke manifest

```powershell
just wpt-run
```

Results are written to `wpt/results/latest.json`.

## 4) Compare against baseline

```powershell
just wpt-diff
```

Use this after engine changes to quickly detect regressions or improvements.

## 5) Update baseline after intentional fixes

```powershell
just wpt-baseline
```

## Result triage

- `pass`: test succeeded and is not listed in expectations.
- `regression`: test failed and is not listed in expectations.
- `expected_fail`: test failed and is listed in `wpt/expectations/known-failures.txt`.
- `unexpected_pass`: test passed but is listed in expectations (candidate to remove from expectations).
- `skipped`: test does not use `testharness.js` and is not handled by this runner yet.

## Notes

- This harness currently targets `testharness.js` style tests.
- Non-testharness tests (e.g., visual/reftest/manual) are marked as `skipped`.
- You can run a focused slice while fixing a bug:

```powershell
just wpt-run filter="querySelector"
```

