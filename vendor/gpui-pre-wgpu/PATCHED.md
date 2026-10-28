# gpui-pre-wgpu 0.3.10, patched for 4x4

This directory is the crates.io package `gpui-pre-wgpu` 0.3.10, the renderer that `gpui-kit`
0.8.1 pulls in through `gpui-pre`. It is Zed's `gpui_wgpu` crate (snapshot of zed@08cbb88),
licensed under the Apache License 2.0 (`LICENSE-APACHE`). The root `Cargo.toml` uses it through
`[patch.crates-io]`.

It was extracted from the published `.crate` file (SHA-256
`ce693915768aae38bf788f245af75aae78e3e1179245860fc80ce831d0a8f74e`, the checksum `Cargo.lock` had
for the registry crate). Two files are modified, as described below; `PATCHED.md` and
`rustfmt.toml` are added. Everything else is exactly as published.

## Why

GPUI asks wgpu for the Vulkan and the OpenGL backends at once (`Backends::VULKAN | Backends::GL`),
and wgpu initializes every backend it is asked for when the instance is created. So even when a
Vulkan adapter draws the window, EGL and Mesa's OpenGL driver (`libgallium`, `libEGL_mesa`,
`libGLdispatch`, `libgbm` and seven driver threads) are loaded and stay loaded.

There is no way to avoid that without changing this crate: GPUI passes the backends explicitly,
so `WGPU_BACKEND` is ignored, and `ZED_DEVICE_ID` only picks an adapter after both driver stacks
are loaded. Upstream tracks the problem in
[zed-industries/zed#63327](https://github.com/zed-industries/zed/issues/63327). The OpenGL
fallback itself must stay: machines without a Vulkan driver (VMs, old GPUs) need it, see
[zed-industries/zed#65960](https://github.com/zed-industries/zed/issues/65960).

Measured on 2026-10-28 (KDE Plasma 6 on Wayland, AMD Radeon 780M with RADV, Mesa 26.2.5; window
on the built-in screen without focus; medians of three runs, MiB), before and after this patch:

| Metric              | Before | After |
| ------------------- | ------ | ----- |
| RSS                 | 150.0  | 110.6 |
| PSS                 | 60.8   | 48.4  |
| USS                 | 51.3   | 41.6  |
| RssAnon (= private) | 17.2   | 12.6  |
| Threads             | 33     | 26    |

The first frame did not change. The [README](../../README.md#performance) puts these numbers in
context; the full investigation is in the owner's local notes.

## What changed

- `src/wgpu_context.rs`: `WgpuContext::instance` asks for Vulkan only, and the new
  `WgpuContext::instance_with_backends` takes the backends to ask for. `create_headless` (only
  with the `test-support` or `bench-support` feature) still asks for Vulkan and OpenGL, so headless
  tests on a machine without Vulkan still find an adapter.
- `src/wgpu_renderer.rs`: the new `create_context_vulkan_first` creates the GPU context in up to
  three attempts, each with its own wgpu instance: a hardware Vulkan adapter; a hardware OpenGL
  adapter; then any adapter, software renderers included, unless the caller rejects them. A
  hardware OpenGL adapter is therefore still preferred over a software Vulkan one (lavapipe), as
  before. The renderer uses it for the first window and, with software adapters rejected as
  before, when it recovers from a lost device. Each failed attempt logs a warning
  (`No usable Backends(VULKAN) adapter (hardware only: true): ...`); if all attempts fail, the error
  lists each of them.
- Both files got a `// Modified for 4x4` line under gpui-pre's own notice (Apache License 2.0,
  section 4(b)).

With Vulkan hidden (`VK_DRIVER_FILES=/nonexistent/icd.json`) the window is drawn through OpenGL
after that warning. With neither Vulkan nor EGL the app fails to open its window, as it did
before (checked during the investigation).

## Guards

- `just lint-policy` fails unless `cargo tree -p x4-app -i gpui-pre-wgpu` resolves to this
  directory, and when Cargo reports the patch as unused. That is what happens when a `gpui-kit`
  bump brings another `gpui-pre-wgpu` version: Cargo only warns and builds the registry crate.
- It also fails when `src/wgpu_renderer.rs` has no `create_context_vulkan_first`, so a copy that was
  re-vendored but not patched again cannot slip through.
- Our gates leave this directory alone: `rustfmt.toml` here disables formatting (for `cargo fmt`,
  the format hook and editors), `_typos.toml` and `.taplo.toml` exclude `vendor/`, `just machete`
  only looks at `crates/`, and clippy only runs on workspace members.

## On a gpui-kit bump

1. Check upstream first. If the new `gpui-pre-wgpu` no longer initializes OpenGL when Vulkan
   works (see zed#63327), delete `vendor/`, the `[patch.crates-io]` entry, lint-policy check (e),
   the `vendor/` exclusions (`_typos.toml`, `.taplo.toml`, `just machete`) and the notes about the
   vendored crate in `AGENTS.md` and `README.md`. Stop here.
2. Bump `gpui-kit` and run `cargo update -p gpui-kit`. Cargo warns that the patch was not used;
   note the new `gpui-pre-wgpu` version `V` and its `checksum` in `Cargo.lock`.
3. Replace the copy with the published crate, keeping `PATCHED.md` and `rustfmt.toml`:

   ```bash
   cargo fetch
   crate=$(ls ~/.cargo/registry/cache/index.crates.io-*/gpui-pre-wgpu-V.crate)
   sha256sum "$crate"   # must equal the checksum from step 2
   tmp=$(mktemp -d)
   cp vendor/gpui-pre-wgpu/PATCHED.md vendor/gpui-pre-wgpu/rustfmt.toml "$tmp"
   rm -rf vendor/gpui-pre-wgpu
   tar xzf "$crate" -C "$tmp"
   mv "$tmp/gpui-pre-wgpu-V" vendor/gpui-pre-wgpu
   mv "$tmp/PATCHED.md" "$tmp/rustfmt.toml" vendor/gpui-pre-wgpu/
   ```

   Run `cargo tree -p x4-app -i gpui-pre-wgpu` without `--locked`, so Cargo updates `Cargo.lock`:
   it shows the `vendor/` path again. `just lint-policy` still fails, because the copy is not
   patched yet. Do not commit yet.
4. Re-apply the patch below. If a hunk does not apply, make the same changes by hand.

   ```bash
   awk '/^```diff$/ { on = 1; next } /^```$/ { on = 0 } on' vendor/gpui-pre-wgpu/PATCHED.md |
       patch -d vendor/gpui-pre-wgpu -p1
   ```

5. Update the version, the checksum and the patch in this file. Run `just check`, then
   `just measure`: it must print `gl_stack_mapped=no` and no GL `WARN:` line. Start the release
   build with `VK_DRIVER_FILES=/nonexistent/icd.json` and check that the window still renders
   (through OpenGL) after the warning.
6. Commit the `gpui-kit` bump, the new copy and the re-applied patch together.

Why one commit: every commit must pass `just check`, and lint-policy refuses an unpatched copy.
A separate "vendored unchanged" commit could only pass if the check had a way to be skipped, and
that would be the same hole the check closes. The patch can still be reviewed on its own: the
commit's diff of "The patch" below shows how the patch changed, and comparing the vendored
directory with the extracted `.crate` (`diff -ru`) shows the whole patch against the new version.

## The patch

Against the published `gpui-pre-wgpu` 0.3.10, paths relative to this directory:

```diff
diff --git a/src/wgpu_context.rs b/src/wgpu_context.rs
index 51cf8b0..e1ae75f 100644
--- a/src/wgpu_context.rs
+++ b/src/wgpu_context.rs
@@ -1,4 +1,5 @@
 // Modified for gpui-pre (snapshot of zed@08cbb88): adapt device queues and sRGB detection to wgpu 30.0.1.
+// Modified for 4x4: create the GPU context Vulkan-first, see PATCHED.md.
 #[cfg(not(target_family = "wasm"))]
 use anyhow::Context as _;
 #[cfg(not(target_family = "wasm"))]
@@ -203,7 +204,8 @@ impl WgpuContext {
         any(test, feature = "bench-support", feature = "test-support")
     ))]
     fn create_headless() -> anyhow::Result<(Self, wgpu::TextureFormat)> {
-        let instance = Self::instance(None);
+        let instance =
+            Self::instance_with_backends(None, wgpu::Backends::VULKAN | wgpu::Backends::GL);
         let device_id_filter = Self::device_id_filter();
         let (adapter, device, queue, dual_source_blending, color_texture_format, target_format) =
             gpui::block_on(async {
@@ -472,8 +474,16 @@ impl WgpuContext {
 
     #[cfg(not(target_family = "wasm"))]
     pub fn instance(display: Option<Box<dyn wgpu::wgt::WgpuHasDisplayHandle>>) -> wgpu::Instance {
+        Self::instance_with_backends(display, wgpu::Backends::VULKAN)
+    }
+
+    #[cfg(not(target_family = "wasm"))]
+    pub fn instance_with_backends(
+        display: Option<Box<dyn wgpu::wgt::WgpuHasDisplayHandle>>,
+        backends: wgpu::Backends,
+    ) -> wgpu::Instance {
         wgpu::Instance::new(wgpu::InstanceDescriptor {
-            backends: wgpu::Backends::VULKAN | wgpu::Backends::GL,
+            backends,
             flags: wgpu::InstanceFlags::default(),
             backend_options: wgpu::BackendOptions::default(),
             memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
diff --git a/src/wgpu_renderer.rs b/src/wgpu_renderer.rs
index f6611df..8557b16 100644
--- a/src/wgpu_renderer.rs
+++ b/src/wgpu_renderer.rs
@@ -1,4 +1,5 @@
 // Modified for gpui-pre (snapshot of zed@08cbb88): adapt device queues and sRGB detection to wgpu 30.0.1.
+// Modified for 4x4: create the GPU context Vulkan-first, see PATCHED.md.
 use crate::{CompositorGpuHint, DeviceErrorState, WgpuAtlas, WgpuContext};
 use anyhow::{Context as _, Result};
 use bytemuck::{Pod, Zeroable};
@@ -299,41 +300,22 @@ impl WgpuRenderer {
     where
         W: HasWindowHandle + HasDisplayHandle + std::fmt::Debug + Send + Sync + Clone + 'static,
     {
-        let window_handle = window
-            .window_handle()
-            .map_err(|e| anyhow::anyhow!("Failed to get window handle: {e}"))?;
-
-        let target = wgpu::SurfaceTargetUnsafe::RawHandle {
-            // Fall back to the display handle already provided via InstanceDescriptor::display.
-            raw_display_handle: None,
-            raw_window_handle: window_handle.as_raw(),
-        };
-
-        // Use the existing context's instance if available, otherwise create a new one.
-        // The surface must be created with the same instance that will be used for
-        // adapter selection, otherwise wgpu will panic.
-        let instance = gpu_context
-            .borrow()
-            .as_ref()
-            .map(|ctx| ctx.instance.clone())
-            .unwrap_or_else(|| WgpuContext::instance(Some(Box::new(window.clone()))));
-
-        // Safety: The caller guarantees that the window handle is valid for the
-        // lifetime of this renderer. In practice, the RawWindow struct is created
-        // from the native window handles and the surface is dropped before the window.
-        let surface = unsafe {
-            instance
-                .create_surface_unsafe(target)
-                .map_err(|e| anyhow::anyhow!("Failed to create surface: {e}"))?
-        };
-
         let mut ctx_ref = gpu_context.borrow_mut();
-        let context = match ctx_ref.as_mut() {
+        let (context, surface) = match ctx_ref.as_mut() {
             Some(context) => {
+                let window_handle = window
+                    .window_handle()
+                    .map_err(|e| anyhow::anyhow!("Failed to get window handle: {e}"))?;
+                let surface = create_surface(&context.instance, window_handle.as_raw())
+                    .context("Failed to create surface")?;
                 context.check_compatible_with_surface(&surface)?;
-                context
+                (context, surface)
+            }
+            None => {
+                let (surface, new_context) =
+                    create_context_vulkan_first(window, compositor_gpu, false)?;
+                (ctx_ref.insert(new_context), surface)
             }
-            None => ctx_ref.insert(WgpuContext::new(instance, &surface, compositor_gpu)?),
         };
 
         let atlas = Arc::new(WgpuAtlas::from_context(context));
@@ -2232,10 +2214,8 @@ impl WgpuRenderer {
             // may need more time to come back (e.g. after suspend/resume).
             std::thread::sleep(std::time::Duration::from_millis(350));
 
-            let instance = WgpuContext::instance(Some(Box::new(window.clone())));
-            let surface = create_surface(&instance, window_handle.as_raw())?;
-            let new_context =
-                WgpuContext::new_rejecting_software(instance, &surface, self.compositor_gpu)?;
+            let (surface, new_context) =
+                create_context_vulkan_first(window, self.compositor_gpu, true)?;
             *gpu_context.borrow_mut() = Some(new_context);
             surface
         } else {
@@ -2544,6 +2524,58 @@ fn instance_range(range: Range<usize>) -> Range<u32> {
     range.start as u32..range.end as u32
 }
 
+/// Creates the shared GPU context without loading driver stacks it does not use.
+///
+/// Asking wgpu for Vulkan and OpenGL at once initializes EGL and maps the OpenGL driver even
+/// when a Vulkan adapter is selected. So try, in order: a hardware Vulkan adapter; a hardware
+/// OpenGL adapter (VMs and old GPUs without a Vulkan driver); and only then any adapter,
+/// software renderers included, unless the caller rejects them. This keeps the old preference
+/// of a hardware OpenGL adapter over a software Vulkan one (lavapipe).
+#[cfg(not(target_family = "wasm"))]
+fn create_context_vulkan_first<W>(
+    window: &W,
+    compositor_gpu: Option<CompositorGpuHint>,
+    reject_software: bool,
+) -> anyhow::Result<(wgpu::Surface<'static>, WgpuContext)>
+where
+    W: HasWindowHandle + HasDisplayHandle + std::fmt::Debug + Send + Sync + Clone + 'static,
+{
+    let raw_window_handle = window
+        .window_handle()
+        .map_err(|e| anyhow::anyhow!("Failed to get window handle: {e}"))?
+        .as_raw();
+    let attempt = |backends: wgpu::Backends,
+                   hardware_only: bool|
+     -> anyhow::Result<(wgpu::Surface<'static>, WgpuContext)> {
+        let instance = WgpuContext::instance_with_backends(Some(Box::new(window.clone())), backends);
+        let surface = create_surface(&instance, raw_window_handle)
+            .context("Failed to create surface")?;
+        let context = if hardware_only {
+            WgpuContext::new_rejecting_software(instance, &surface, compositor_gpu)?
+        } else {
+            WgpuContext::new(instance, &surface, compositor_gpu)?
+        };
+        Ok((surface, context))
+    };
+    let mut attempts = vec![(wgpu::Backends::VULKAN, true), (wgpu::Backends::GL, true)];
+    if !reject_software {
+        attempts.push((wgpu::Backends::VULKAN | wgpu::Backends::GL, false));
+    }
+    let mut errors = Vec::new();
+    for (backends, hardware_only) in attempts {
+        match attempt(backends, hardware_only) {
+            Ok(created) => return Ok(created),
+            Err(error) => {
+                log::warn!(
+                    "No usable {backends:?} adapter (hardware only: {hardware_only}): {error:#}"
+                );
+                errors.push(format!("{backends:?}: {error:#}"));
+            }
+        }
+    }
+    anyhow::bail!("No GPU adapter can drive this window: {}", errors.join("; "))
+}
+
 #[cfg(not(target_family = "wasm"))]
 fn create_surface(
     instance: &wgpu::Instance,
```
