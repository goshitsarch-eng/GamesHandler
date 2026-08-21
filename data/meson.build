app_id = 'org.gamehandler.GameHandler'

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
