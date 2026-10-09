import 'dart:io';

import 'package:path/path.dart' as p;

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:desktop_drop/desktop_drop.dart';
import 'package:file_selector/file_selector.dart';
import 'package:url_launcher/url_launcher.dart';
import 'package:window_manager/window_manager.dart';

import '../models/game.dart';
import '../state/library_state.dart';
import '../ui/game_editor.dart';
import '../ui/library.dart';
import '../ui/catalogue.dart';
import '../ui/settings.dart';

class GameHandlerApp extends StatefulWidget {
  const GameHandlerApp({
    super.key,
    required this.state,
    this.nativeMenus = true,
  });
  final LibraryState state;
  final bool nativeMenus;
  @override
  State<GameHandlerApp> createState() => _GameHandlerAppState();
}

class _GameHandlerAppState extends State<GameHandlerApp> {
  final navigator = GlobalKey<NavigatorState>();
  final messenger = GlobalKey<ScaffoldMessengerState>();
  final searchFocus = FocusNode();
  int page = 0;
  LibraryState get state => widget.state;
  int _artworkRevision = 0;

  @override
  void initState() {
    super.initState();
    _artworkRevision = state.artworkRevision;
    state.addListener(_refreshArtwork);
  }

  void _refreshArtwork() {
    if (_artworkRevision == state.artworkRevision) return;
    _artworkRevision = state.artworkRevision;
    PaintingBinding.instance.imageCache.clear();
    PaintingBinding.instance.imageCache.clearLiveImages();
  }

  @override
  void dispose() {
    state.removeListener(_refreshArtwork);
    searchFocus.dispose();
    super.dispose();
  }

  Future<void> action(Future<void> Function() operation) async {
    try {
      await operation();
    } catch (e) {
      messenger.currentState?.showSnackBar(
        SnackBar(
          content: Text(e.toString()),
          duration: const Duration(seconds: 8),
        ),
      );
    }
  }

  Future<void> edit([GameDto? game, String? path]) async {
    await action(() async {
      final value =
          game?.fields ??
          Map<String, dynamic>.from(await state.backend.call('new_game'));
      if (!mounted) return;
      await showDialog<void>(
        context: navigator.currentContext!,
        builder: (context) =>
            GameEditor(state: state, initial: {...value, 'exe_path': ?path}),
      );
    });
  }

  Future<void> remove(GameDto game) async {
    final confirmed = await showDialog<bool>(
      context: navigator.currentContext!,
      builder: (c) => AlertDialog(
        title: Text('Remove ${game.name}?'),
        content: const Text(
          'This removes the library entry. Game files and compatibility prefixes are kept.',
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(c, false),
            child: const Text('Cancel'),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(c, true),
            child: const Text('Remove'),
          ),
        ],
      ),
    );
    if (confirmed == true) await state.remove(game.id);
  }

  Future<void> play(GameDto game) async {
    await state.launch(game.id);
    if (state.settings['close_on_launch'] == true) await windowManager.hide();
    messenger.currentState?.showSnackBar(
      SnackBar(content: Text('Launched ${game.name}')),
    );
  }

  Future<void> gameAction(GameDto game, String command) async {
    switch (command) {
      case 'play':
        await play(game);
      case 'edit':
        await edit(game);
      case 'remove':
        await remove(game);
      case 'fetch_cover':
        await state.startJob('cover', {'id': game.id});
      case 'custom_cover':
        final f = await openFile(
          acceptedTypeGroups: [
            const XTypeGroup(
              label: 'Images',
              extensions: ['png', 'jpg', 'jpeg', 'webp', 'gif', 'ico'],
            ),
          ],
        );
        if (f != null) {
          await state.backend.call('import_cover', {
            'id': game.id,
            'path': f.path,
          });
          state.invalidateArtwork();
          await state.refresh();
        }
      case 'winecfg':
      case 'winetricks':
        await state.backend.call('prefix_tool', {
          'id': game.id,
          'tool': command,
        });
      case 'prefix':
        final path =
            await state.backend.call('prefix_path', {'id': game.id}) as String;
        if (!await launchUrl(
          Uri.directory(path),
          mode: LaunchMode.externalApplication,
        )) {
          throw StateError('Could not open the prefix folder');
        }
      case 'shortcut':
        final path = await state.backend.call('shortcut', {
          'id': game.id,
          'executable': p.join(
            p.dirname(Platform.resolvedExecutable),
            Platform.isWindows ? 'gamehandler-cli.exe' : 'gamehandler-cli',
          ),
        });
        messenger.currentState?.showSnackBar(
          SnackBar(content: Text('Created shortcut: $path')),
        );
    }
  }

  void focusSearch() {
    if (page == 0) {
      searchFocus.requestFocus();
      return;
    }
    setState(() => page = 0);
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) searchFocus.requestFocus();
    });
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: state,
    builder: (context, _) {
      final mode = switch (state.settings['color_scheme']) {
        'light' => ThemeMode.light,
        'dark' => ThemeMode.dark,
        _ => ThemeMode.system,
      };
      return _platformMenus(
        MaterialApp(
          navigatorKey: navigator,
          scaffoldMessengerKey: messenger,
          title: 'GameHandler',
          debugShowCheckedModeBanner: false,
          themeMode: mode,
          theme: ThemeData(
            colorScheme: ColorScheme.fromSeed(
              seedColor: const Color(0xff168b91),
            ),
            useMaterial3: true,
            visualDensity: VisualDensity.compact,
          ),
          darkTheme: ThemeData(
            colorScheme: ColorScheme.fromSeed(
              seedColor: const Color(0xff67d5da),
              brightness: Brightness.dark,
            ),
            useMaterial3: true,
            visualDensity: VisualDensity.compact,
          ),
          home: Builder(
            builder: (context) => CallbackShortcuts(
              bindings: {
                SingleActivator(
                  LogicalKeyboardKey.keyN,
                  control: !Platform.isMacOS,
                  meta: Platform.isMacOS,
                ): () =>
                    edit(),
                SingleActivator(
                  LogicalKeyboardKey.keyF,
                  control: !Platform.isMacOS,
                  meta: Platform.isMacOS,
                ): focusSearch,
                SingleActivator(
                  LogicalKeyboardKey.comma,
                  control: !Platform.isMacOS,
                  meta: Platform.isMacOS,
                ): () =>
                    setState(() => page = 5),
                SingleActivator(
                  LogicalKeyboardKey.keyQ,
                  control: !Platform.isMacOS,
                  meta: Platform.isMacOS,
                ): () =>
                    windowManager.close(),
              },
              child: Focus(
                autofocus: true,
                child: Scaffold(
                  appBar: AppBar(
                    title: const Text('GameHandler'),
                    actions: [
                      IconButton(
                        tooltip: 'Add game',
                        onPressed: () => edit(),
                        icon: const Icon(Icons.add),
                      ),
                      IconButton(
                        tooltip: 'Refresh',
                        onPressed: () => action(() async {
                          await state.refresh();
                          await state.reloadCatalogue();
                        }),
                        icon: const Icon(Icons.refresh),
                      ),
                    ],
                  ),
                  body: Column(
                    children: [
                      if (!Platform.isMacOS)
                        MenuBar(
                          children: [
                            SubmenuButton(
                              menuChildren: [
                                MenuItemButton(
                                  onPressed: () => edit(),
                                  child: const Text('Add game'),
                                ),
                                MenuItemButton(
                                  onPressed: () => windowManager.close(),
                                  child: const Text('Quit'),
                                ),
                              ],
                              child: const Text('Library'),
                            ),
                            SubmenuButton(
                              menuChildren: [
                                MenuItemButton(
                                  onPressed: focusSearch,
                                  child: const Text('Search'),
                                ),
                                MenuItemButton(
                                  onPressed: () => setState(() => page = 5),
                                  child: const Text('Settings'),
                                ),
                              ],
                              child: const Text('View'),
                            ),
                          ],
                        ),
                      if (state.error != null)
                        MaterialBanner(
                          content: Text(state.error!),
                          actions: [
                            TextButton(
                              onPressed: () => action(state.refresh),
                              child: const Text('Retry'),
                            ),
                          ],
                        ),
                      if (state.loading) const LinearProgressIndicator(),
                      Expanded(
                        child: Row(
                          children: [
                            NavigationRail(
                              scrollable: true,
                              selectedIndex: page,
                              labelType: NavigationRailLabelType.all,
                              onDestinationSelected: (i) =>
                                  setState(() => page = i),
                              destinations: const [
                                NavigationRailDestination(
                                  icon: Icon(Icons.sports_esports_outlined),
                                  label: Text('Library'),
                                ),
                                NavigationRailDestination(
                                  icon: Icon(Icons.install_desktop),
                                  label: Text('Installers'),
                                ),
                                NavigationRailDestination(
                                  icon: Icon(Icons.download_outlined),
                                  label: Text('Runners'),
                                ),
                                NavigationRailDestination(
                                  icon: Icon(Icons.extension_outlined),
                                  label: Text('Plugins'),
                                ),
                                NavigationRailDestination(
                                  icon: Icon(Icons.info_outline),
                                  label: Text('About'),
                                ),
                                NavigationRailDestination(
                                  icon: Icon(Icons.settings_outlined),
                                  label: Text('Settings'),
                                ),
                              ],
                            ),
                            const VerticalDivider(width: 1),
                            Expanded(
                              child: DropTarget(
                                onDragDone: (details) {
                                  if (details.files.isNotEmpty) {
                                    edit(null, details.files.first.path);
                                  }
                                },
                                child: switch (page) {
                                  0 => LibraryPage(
                                    state: state,
                                    searchFocus: searchFocus,
                                    onAdd: () => edit(),
                                    onAction: (g, c) =>
                                        action(() => gameAction(g, c)),
                                  ),
                                  1 => InstallersPage(
                                    state: state,
                                    onAction: action,
                                  ),
                                  2 => RunnersPage(
                                    state: state,
                                    onAction: action,
                                  ),
                                  3 => PluginsPage(
                                    state: state,
                                    onAction: action,
                                  ),
                                  4 => AboutPage(
                                    state: state,
                                    onAction: action,
                                  ),
                                  _ => SettingsPage(
                                    state: state,
                                    onAction: action,
                                  ),
                                },
                              ),
                            ),
                          ],
                        ),
                      ),
                      if (state.jobs.isNotEmpty)
                        ConstrainedBox(
                          constraints: const BoxConstraints(maxHeight: 190),
                          child: ListView(
                            shrinkWrap: true,
                            children: state.jobs
                                .map((job) => _job(job, context))
                                .toList(),
                          ),
                        ),
                    ],
                  ),
                ),
              ),
            ),
          ),
        ),
      );
    },
  );

  Widget _platformMenus(Widget child) =>
      (!Platform.isMacOS || !widget.nativeMenus)
      ? child
      : PlatformMenuBar(
          menus: [
            PlatformMenu(
              label: 'GameHandler',
              menus: [
                PlatformMenuItem(
                  label: 'About GameHandler',
                  onSelected: () => setState(() => page = 4),
                ),
                PlatformMenuItem(
                  label: 'Settings…',
                  shortcut: const SingleActivator(
                    LogicalKeyboardKey.comma,
                    meta: true,
                  ),
                  onSelected: () => setState(() => page = 5),
                ),
                const PlatformProvidedMenuItem(
                  type: PlatformProvidedMenuItemType.servicesSubmenu,
                ),
                const PlatformProvidedMenuItem(
                  type: PlatformProvidedMenuItemType.hide,
                ),
                const PlatformProvidedMenuItem(
                  type: PlatformProvidedMenuItemType.hideOtherApplications,
                ),
                const PlatformProvidedMenuItem(
                  type: PlatformProvidedMenuItemType.showAllApplications,
                ),
                PlatformMenuItem(
                  label: 'Quit GameHandler',
                  shortcut: const SingleActivator(
                    LogicalKeyboardKey.keyQ,
                    meta: true,
                  ),
                  onSelected: () => windowManager.close(),
                ),
              ],
            ),
            PlatformMenu(
              label: 'Library',
              menus: [
                PlatformMenuItem(
                  label: 'Add game…',
                  shortcut: const SingleActivator(
                    LogicalKeyboardKey.keyN,
                    meta: true,
                  ),
                  onSelected: () => edit(),
                ),
                PlatformMenuItem(
                  label: 'Search',
                  shortcut: const SingleActivator(
                    LogicalKeyboardKey.keyF,
                    meta: true,
                  ),
                  onSelected: focusSearch,
                ),
              ],
            ),
          ],
          child: child,
        );

  Widget _job(Map<String, dynamic> job, BuildContext context) {
    final status = job['status'] as String;
    final running = status == 'running';
    final fraction = (job['progress'] as num).toDouble();
    final message =
        job['error']?['message'] ?? job['result']?['message'] ?? status;
    return ListTile(
      title: Text('${job['kind']}: $message'),
      subtitle: running
          ? LinearProgressIndicator(
              value: fraction < 0 ? null : fraction.clamp(0, 1),
            )
          : null,
      trailing: Wrap(
        children: [
          if (status == 'needs_executable')
            TextButton(
              onPressed: () => action(() async {
                final f = await openFile();
                if (f != null) {
                  await state.backend.call('complete_install', {
                    'job': job['id'],
                    'path': f.path,
                  });
                  await state.pollJobs();
                }
              }),
              child: const Text('Locate executable'),
            ),
          if (!running || job['kind'] != 'launch')
            TextButton(
              onPressed: () => action(() async {
                await state.backend.call(
                  running ? 'cancel_job' : 'dismiss_job',
                  {'id': job['id']},
                );
                await state.pollJobs();
              }),
              child: Text(running ? 'Cancel' : 'Dismiss'),
            ),
        ],
      ),
    );
  }
}
