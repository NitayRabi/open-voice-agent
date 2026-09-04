# shellcheck shell=bash
# Source this before building against the local sysroot from
# setup-build-sysroot.sh:   source ./scripts/build-env.sh
# No-op-safe to source on a machine with system -devel packages if you unset
# OVA_SYSROOT first.

OVA_SYSROOT="${OVA_SYSROOT:-$HOME/.cache/ova-build-sysroot}"

if [ -d "$OVA_SYSROOT/usr/lib64/pkgconfig" ]; then
  export PKG_CONFIG_SYSROOT_DIR="$OVA_SYSROOT"
  export PKG_CONFIG_PATH="$OVA_SYSROOT/usr/lib64/pkgconfig:$OVA_SYSROOT/usr/share/pkgconfig"
  export PKG_CONFIG_LIBDIR="$OVA_SYSROOT/usr/lib64/pkgconfig:$OVA_SYSROOT/usr/share/pkgconfig"
  export LD_LIBRARY_PATH="$OVA_SYSROOT/usr/lib64${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
  # link against the sysroot .so files and record an rpath-link so the linker
  # can resolve their transitive DT_NEEDED without polluting the final rpath.
  export RUSTFLAGS="-L $OVA_SYSROOT/usr/lib64 -C link-arg=-Wl,-rpath-link,$OVA_SYSROOT/usr/lib64${RUSTFLAGS:+ $RUSTFLAGS}"
  echo "build-env: using sysroot $OVA_SYSROOT"
else
  echo "build-env: no sysroot at $OVA_SYSROOT -- assuming system -devel packages"
fi
