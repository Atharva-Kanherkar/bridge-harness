# macOS crash analysis

Bridge's native audit found concrete launch and window-lifecycle risks. These
are separate from proving the cause of every historical crash. A report's
timestamp, version, executable path, and recorded signing identity identify
which build failed; an old Problem Report does not establish that a newly
installed build crashed.

## Verified code paths and fixes

- **Fallible startup could become an abort.** Tauri 2.11.5 turns an error from
  setup into a Rust panic while handling its Ready event. On macOS this can
  escape tao's `extern "C"` launch callback, which cannot unwind. Bridge now
  handles startup failures explicitly and records them. Native window creation
  must also occur inside that handled path: Tauri creates ordinary configured
  windows before calling the application's setup hook. See
  [the shell startup code](../src-tauri/src/lib.rs) and
  [Rust's FFI unwinding rules](https://doc.rust-lang.org/nomicon/ffi.html#ffi-and-unwinding).
- **There were two owners of traffic-light placement.** Bridge moved the
  buttons itself while `trafficLightPosition` enabled tao 0.35.3's redraw-time
  titlebar mutation, including unchecked button/superview lookups. The window
  configuration and Bridge now leave those controls to AppKit.
- **Chrome called a private selector and could mutate layout inline.**
  `window-vibrancy` 0.6.0 calls the private `NSVisualEffectView.setCornerRadius:`
  selector. Bridge now creates one effect view using public AppKit APIs and
  rounds its layer using `CALayer`. Changes are idempotent and dispatched
  asynchronously to the main queue. Tauri's `run_on_main_thread` executes
  immediately when already on the main thread, so it did not provide that
  deferral. See [window chrome](../src-tauri/src/window_chrome.rs) and
  [Apple's main-thread requirements for NSView](https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/Multithreading/ThreadSafetySummary/ThreadSafetySummary.html).
- **A release-build zoom reproduced an over-release of Wry's parent view.**
  On this Mac, AppKit first raised `NSInternalInconsistencyException` because
  `WryWebViewParent0.55.1` reached deallocation while still attached to a
  superview. Continuing the damaged view's lifecycle then triggered
  `Cannot form weak reference`. The captured exception stack and ARM64
  disassembly placed the first failing release at the temporary `contentView`
  drop in Bridge's resize handler. Adding reference-count tracing changed the
  reproduction, and the static getters/iterators have balanced ownership; this
  does not establish a general Rust, objc2, or AppKit defect. Bridge now owns its
  material explicitly on the main thread and updates only that material during
  resizing. It no longer retrieves, traverses, or retains/releases Wry's parent
  in the resize path. Startup takes one balanced explicit content-view retain;
  window destruction releases Bridge's cached material ownership.
- **Multiple builds could disrupt the same daemon.** An incompatible daemon
  now produces an actionable error instead of being killed by a new launcher.
  An atomic desktop lease closes the simultaneous-launch race before backend
  startup. Long Unix socket paths fail validation before daemon/store startup,
  with guidance to shorten `BRIDGE_DATA_DIR`.
- **Finder launches lacked durable failure evidence.** The native diagnostics
  log now records startup identity, panic backtraces, and native exception
  call stacks with rotation. It is
  separate from the filtered report analyzer below.

## What a stack establishes

| Signature | Supported interpretation | Not established by this alone |
| --- | --- | --- |
| `panic_cannot_unwind` | A Rust panic reached a boundary that cannot unwind | An Objective-C exception, or the original panic reason |
| `__rust_foreign_exception` | A foreign exception reached Rust's panic handling | Which foreign language/call caused it, or a JIT failure |
| `_objc_fatal` with `weak_register_no_lock` / `objc_initWeak` | An Objective-C weak-reference operation aborted | The identity of the dying object or a particular view's responsibility |
| `SIGABRT` alone | The process aborted | Any specific root cause |

Apple's runtime explicitly aborts when registering a weak reference to an
object that is deallocating or rejects weak references. The historical handoff's
Auto Layout stack is consistent with that failure class; it does not prove
that vibrancy, a corner radius, or traffic lights caused it.
[Apple runtime source](https://github.com/apple-oss-distributions/objc4/blob/main/runtime/objc-weak.mm)

`objc2::exception::catch` catches Objective-C exceptions, not direct aborts, and
lets Rust panics propagate. Removing risky mutations therefore addresses more
than adding another exception wrapper. JIT entitlements remain a release
configuration requirement here; neither an abort stack nor a reported JIT
memory region alone proves a historical JIT cause or fix.
[objc2 documentation](https://docs.rs/objc2/latest/objc2/exception/fn.catch.html)

## Analyze one selected report

```sh
python3 scripts/analyze-macos-crash.py /absolute/path/to/bridge-deck-report.ips
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s scripts/test -p 'test_*.py'
```

The script accepts JSON `.ips` reports, including the metadata-plus-body format.
It reads only the explicitly supplied file, performs no network requests, and
prints an allowlisted JSON summary: timestamp, version, executable, recorded
signing identity/status, crash classification, and up to 20 faulting-thread
frames. It does not print application-specific diagnostics, memory summaries,
environment details, or other threads. Missing symbols remain unknown; the
script does not symbolicate, verify signatures, or check notarization. An empty
Team ID is reported as absent, without inferring ad-hoc signing.

Fixtures are synthetic. For validation, exercise launch, resize, fullscreen,
theme changes, and sustained work against the exact signed artifact. Record
that artifact's version and signature with the result; a successful smoke test
does not prove every historical crash is resolved.
