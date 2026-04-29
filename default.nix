{ lib
, stdenv
, fetchFromGitHub
, rustPlatform
, pnpm_10
, fetchPnpmDeps
, nodejs_20
, cargo
, rustc
, pkg-config
, wrapGAppsHook3
, at-spi2-atk
, atkmm
, cairo
, gdk-pixbuf
, glib
, gtk3
, harfbuzz
, librsvg
, libsoup_3
, pango
, webkitgtk_4_1
, openssl
, libayatana-appindicator
, dbus
}:

# Pebble —— Tauri 2 + pnpm + Rust workspace 的桌面邮件客户端。
#
# 上游的 `pnpm build` 会走 `tauri build`，而 Tauri bundler 在 Nix 沙盒里经常会
# 因为联网/跨平台 bundle 资源失败；Pebble 自己还把 bundle target 设成了 `nsis`。
# 这里统一拆成两步：
#   1. `pnpm run build:frontend` 只产出前端 dist/
#   2. `cargo build -p pebble --release --frozen` 只编译 Tauri 二进制
#
# Pebble 是 Cargo workspace，`Cargo.toml`/`Cargo.lock` 都在仓库根目录，不在
# `src-tauri/` 下，所以 Rust vendoring 也必须按 workspace 根目录来配。

stdenv.mkDerivation (finalAttrs: {
  pname = "pebble";
  version = "0.0.1";

  src = fetchFromGitHub {
    owner = "QingJ01";
    repo = "Pebble";
    rev = "v${finalAttrs.version}";
    hash = "sha256-p5eRtHJ89pTlQHxATd4WyXWT0b3a7eGS05agNo4YFbg=";
  };

  pnpmDeps = fetchPnpmDeps {
    inherit (finalAttrs) pname version src;
    fetcherVersion = 2;
    hash = "sha256-jp+OGytRpHFUbfclDAW1V8dwLpaInw3KTAQMXvqcsEA=";
  };

  cargoDeps = rustPlatform.importCargoLock {
    lockFile = "${finalAttrs.src}/Cargo.lock";
  };

  nativeBuildInputs = [
    nodejs_20
    pnpm_10.configHook
    rustPlatform.cargoSetupHook
    cargo
    rustc
    pkg-config
    wrapGAppsHook3
  ];

  buildInputs = [
    at-spi2-atk
    atkmm
    cairo
    gdk-pixbuf
    glib
    gtk3
    harfbuzz
    librsvg
    libsoup_3
    pango
    webkitgtk_4_1
    openssl
    libayatana-appindicator
    dbus
  ];

  buildPhase = ''
    runHook preBuild

    export HOME=$(mktemp -d)

    substituteInPlace src-tauri/tauri.conf.json \
      --replace-fail '"devUrl": "http://127.0.0.1:1420",' ""

    pnpm run build:frontend

    cargo build --package pebble --release --frozen

    runHook postBuild
  '';

  installPhase = ''
    runHook preInstall

    install -Dm755 target/release/pebble $out/bin/pebble

    # 图标
    for size in 32x32 128x128; do
      install -Dm644 src-tauri/icons/''${size}.png \
        $out/share/icons/hicolor/''${size}/apps/pebble.png
    done
    install -Dm644 src-tauri/icons/128x128@2x.png \
      $out/share/icons/hicolor/256x256/apps/pebble.png


    install -Dm644 /dev/stdin $out/share/applications/pebble.desktop <<EOF
    [Desktop Entry]
    Type=Application
    Name=Pebble
    GenericName=Pebble
    Comment=A local-first desktop email client for people who want a calmer, more private inbox.
    Exec=pebble %U
    Icon=pebble
    Terminal=false
    Categories=Network;Email;
    StartupWMClass=Pebble
    EOF

    runHook postInstall
  '';

  preFixup = ''
    gappsWrapperArgs+=(
      --prefix LD_LIBRARY_PATH : "${lib.makeLibraryPath [ libayatana-appindicator ]}"
      --set-default GDK_BACKEND x11
      --set-default WEBKIT_DISABLE_DMABUF_RENDERER 1
      --set-default WEBKIT_DISABLE_COMPOSITING_MODE 1
    )
  '';

  meta = with lib; {
    description = "A local-first desktop email client for people who want a calmer, more private inbox.";
    homepage = "https://github.com/QingJ01/Pebble";
    license = licenses.agpl3Only;
    mainProgram = "pebble";
    platforms = platforms.linux;
  };
})
