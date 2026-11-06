// zap.ts — converge a Debian/Wayland system to have Servo's build toolchain.
//
// Zervo embeds Servo (the `servo` crate, built from source via Cargo),
// which compiles SpiderMonkey + WebRender + stylo and needs a native build
// toolchain. This config installs that toolchain so `cargo build -p zervo`
// can succeed. The package set mirrors Servo's own apt prerequisites
// (servo/python/servo/platform/linux_packages/apt/apt_common.txt) plus
// python3-mako (stylo's build scripts run mako for CSS property codegen).
//
//   zap apply            # dry-run (preview)
//   zap apply --wetrun   # converge (install)
//
// Debian target. Servo's Ubuntu-only extra (libgstreamer-plugins-good1.0-dev)
// is omitted — add it on Ubuntu if the build complains.

// Type-only import — erased at eval time, so the evaluator needs no module
// resolution; it buys the typed `ZapBuilder` for authoring.
import type {ZapBuilder} from '@fuzdev/zap/zap.ts';

export default (zap: ZapBuilder): void => {
  zap.target({
    local: true,
    package_manager: 'apt',
    service_manager: 'systemd',
  });

  // Servo build toolchain: mirrors servo's apt_common.txt except for the one
  // rename noted below; id === apt name.
  const apt_pkgs = [
    'build-essential', 'ca-certificates', 'ccache', 'clang', 'cmake', 'curl',
    'g++', 'git', 'gperf', 'libdbus-1-dev',
    // deliberately differs from Servo's list: its `libfreetype6-dev` is only a
    // virtual package name on Debian 13, so it never reads as installed
    'libfreetype-dev',
    'libgl1-mesa-dri', 'libgles2-mesa-dev', 'libglib2.0-dev',
    'gstreamer1.0-plugins-good', 'gstreamer1.0-plugins-bad',
    'libgstreamer-plugins-bad1.0-dev', 'gstreamer1.0-plugins-ugly',
    'gstreamer1.0-plugins-base', 'libgstreamer-plugins-base1.0-dev',
    'gstreamer1.0-libav', 'libgstrtspserver-1.0-dev', 'gstreamer1.0-tools',
    'libges-1.0-dev', 'libharfbuzz-dev', 'liblzma-dev', 'libudev-dev',
    'libunwind-dev', 'libvulkan1', 'libx11-dev', 'libxcb-render0-dev',
    'libxcb-shape0-dev', 'libxcb-xfixes0-dev', 'libxmu-dev', 'libxmu6',
    'libegl1-mesa-dev', 'llvm-dev', 'm4', 'xorg-dev', 'libxkbcommon0',
    'libxkbcommon-x11-0',
    'python3-mako', // stylo build codegen
  ];

  for (const pkg of apt_pkgs) {
    zap.pkg({id: pkg, name: pkg, from: {apt: pkg}});
  }
};
