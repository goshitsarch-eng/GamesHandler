project(
  'gamehandler',
  version: '0.7.1',
  license: 'GPL-3.0-or-later',
  meson_version: '>= 0.64.0',
)

python = import('python')
# PySide6 and the Kirigami QML modules are runtime dependencies; they are
# deliberately not required at build time so packaging stays headless.
py = python.find_installation('python3')

subdir('gamehandler')
subdir('bin')
subdir('data')
