# Installs the shared metadata for the PYTHON application (0.7.x), via the
# top-level meson.build. It is deliberately kept: the Python tree is the parity
# reference on this branch (DECISIONS D-02), its install is still exercised by
# `meson compile`, and tests/test_packaging.py reads this file.
#
# The Rust application (0.8.0) does NOT use Meson. Its Flatpak manifest installs
# the same three files straight from this directory with `install -Dm0644`
# (build-aux/flatpak/com.goshapps.GameHandler.json:94-96), and
# scripts/verify.sh's desktop-metainfo stage validates them with
# desktop-file-validate and appstreamcli — so the files below are the single
# source for both builds and must stay in step.
#
# The LICENSE install below **does** have a counterpart in the Rust manifest:
# `install -Dm0644 LICENSE ${FLATPAK_DEST}/share/licenses/com.goshapps.GameHandler/LICENSE`
# (build-aux/flatpak/com.goshapps.GameHandler.json:101). That line was added in
# `8abac0b` (PKG-01) alongside the osslsigncode and DXVK licence texts, so the
# regression this comment used to assert is closed, and PKG-08's own evidence
# (`grep LICENSE` over the manifest finding no application COPYING and no
# `"LICENSE"` path) described the pre-fix manifest. The consequence for the day
# the Python tree is retired is therefore only this: removing *this* file must
# not be taken to remove the licence from the Flatpak, because the manifest line
# is what puts it there now. `PKG-08` in docs/audit/PACKAGING.md carries the
# full measurement.
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
# Raster sizes for the consumers that cannot read SVG (PKG-05). The master
# artwork is an SVG and the PNGs beside it are rendered from it, so the two
# directories must be installed together or a size goes missing.
foreach size : ['64x64', '128x128', '256x256', '512x512']
  install_data(
    'icons/hicolor' / size / 'apps' / app_id + '.png',
    install_dir: get_option('datadir') / 'icons/hicolor' / size / 'apps',
  )
endforeach
