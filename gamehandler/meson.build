sources = [
  '__init__.py',
  '__main__.py',
  'main.py',
  'bridge.py',
  'theme.py',
  'models.py',
  'config.py',
  'settings.py',
  'covers.py',
  'exe_icons.py',
  'credits.py',
  'netpaths.py',
  'plugins.py',
  'runners.py',
  'installers.py',
]

py.install_sources(sources, subdir: 'gamehandler')

qml_sources = [
  'qml/Main.qml',
  'qml/LibraryPage.qml',
  'qml/GameFormPage.qml',
  'qml/InstallersPage.qml',
  'qml/RunnersPage.qml',
  'qml/PluginsPage.qml',
  'qml/CreditsPage.qml',
  'qml/SettingsPage.qml',
  'qml/CoverArt.qml',
]

py.install_sources(qml_sources, subdir: 'gamehandler', preserve_path: true)

install_data(
  '../data/microsoft-identity-verification-root-ca-2020.pem',
  install_dir: py.get_install_dir() / 'gamehandler',
)
