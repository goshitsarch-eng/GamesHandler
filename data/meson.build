# Installs the shared metadata for the PYTHON application (0.7.x), via the
# top-level meson.build. It is deliberately kept: the Python tree is the parity
# reference on this branch (DECISIONS D-02), its install is still exercised by
# `meson compile`, and tests/test_packaging.py reads this file.
#
# The Rust application (0.8.0) does NOT use Meson. Its Flatpak manifest installs
# the same three files straight from this directory with `install -Dm0644`
# (build-aux/flatpak/com.goshapps.GameHandler.json:91-93), and
# scripts/verify.sh's desktop-metainfo stage validates them with
# desktop-file-validate and appstreamcli — so the files below are the single
# source for both builds and must stay in step.
#
# One difference to be aware of when the Python tree is eventually retired: the
# LICENSE install below has no counterpart in the Rust manifest, which installs
# only the osslsigncode and DXVK licence texts. Removing this file without
# replacing that install would drop the application's own GPL text from the
# Flatpak.
app_id = 'com.goshapps.GameHandler'

install_data(
  meson.project_source_root() / 'LICENSE',
  install_dir: get_option('datadir') / 'licenses' / app_id,
)

# Desktop entry
desktop_file = app_id + '.desktop'
install_data(
  desktop_file,
  install_dir: get_option('datadir') / 'applications',
)

desktop_utils = find_program('desktop-file-validate', required: false)
if desktop_utils.found()
  test('validate-desktop', desktop_utils, args: [files(desktop_file)])
endif

# AppStream metainfo
metainfo_file = app_id + '.metainfo.xml'
install_data(
  metainfo_file,
  install_dir: get_option('datadir') / 'metainfo',
)

appstream_util = find_program('appstreamcli', 'appstream-util', required: false)
if appstream_util.found()
  test(
    'validate-metainfo',
    appstream_util,
    args: ['validate', '--no-net', files(metainfo_file)],
  )
endif

# Icon
install_data(
  'icons/hicolor/scalable/apps' / app_id + '.svg',
  install_dir: get_option('datadir') / 'icons/hicolor/scalable/apps',
)
