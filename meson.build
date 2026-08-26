project(
  'gamehandler',
  version: '0.6.1',
  license: 'GPL-3.0-or-later',
  meson_version: '>= 0.62.0',
)

python = import('python')
py = python.find_installation('python3', modules: ['gi'])

subdir('gamehandler')
subdir('bin')
subdir('data')

gnome = import('gnome')
gnome.post_install(
  gtk_update_icon_cache: true,
  update_desktop_database: true,
)
