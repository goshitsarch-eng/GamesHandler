import 'dart:io';
import 'dart:async';
import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:path/path.dart' as p;
import 'package:path_provider/path_provider.dart';
import 'package:window_manager/window_manager.dart';

import 'bridge/backend.dart';
import 'state/library_state.dart';
import 'app/app.dart';

Future<void> main(List<String> arguments) async {
  WidgetsFlutterBinding.ensureInitialized();
  final directories = await applicationDirectories();
  final backend = Backend(config: directories.$1, data: directories.$2);
  if (arguments.contains('--version')) {
    stdout.writeln('GameHandler 0.9.0-alpha.1');
    exit(0);
  }
  if (arguments.contains('--list')) {
    final value = await backend.call('snapshot', {
      'query': '',
      'category': 'All',
      'sort': 'name',
    });
    for (final game in value['games']) {
      stdout.writeln('${game['id']}\t${game['name']}');
    }
    exit(0);
  }
  final launch = arguments.indexOf('--launch');
  if (launch >= 0) {
    try {
      if (launch + 1 >= arguments.length) {
        throw const BackendFailure('validation', 'Specify a game id');
      }
      await backend.call('launch', {'id': arguments[launch + 1]});
      exit(0);
    } catch (e) {
      stderr.writeln(e);
      exit(1);
    }
  }
  await windowManager.ensureInitialized();
  final window = WindowPreferences(File(p.join(directories.$1, 'window.json')));
  final size = await window.load();
  windowManager.addListener(window);
  await windowManager.setPreventClose(true);
  final state = LibraryState(backend);
  window.beforeClose = state.dispose;
  var seenLaunchFailure = state.launchFailureVersion;
  state.addListener(() {
    if (state.launchFailureVersion != seenLaunchFailure) {
      seenLaunchFailure = state.launchFailureVersion;
      unawaited(
        windowManager
            .restore()
            .then((_) => windowManager.show())
            .then((_) => windowManager.focus()),
      );
    }
  });
  runApp(GameHandlerApp(state: state));
  await windowManager.waitUntilReadyToShow(
    WindowOptions(
      size: size,
      minimumSize: Size(720, 480),
      title: 'GameHandler',
    ),
    () async {
      await windowManager.show();
      await windowManager.focus();
    },
  );
  if (Platform.isLinux) {
    final icon = p.join(
      p.dirname(Platform.resolvedExecutable),
      'share',
      'gamehandler',
      'app_icon.png',
    );
    if (await File(icon).exists()) {
      await windowManager.setIcon(icon);
    }
  }
  await state.initialize();
}

Future<(String, String)> applicationDirectories() async {
  final env = Platform.environment;
  final configOverride = env['GAMEHANDLER_CONFIG_HOME'];
  final dataOverride = env['GAMEHANDLER_DATA_HOME'];
  if (Platform.isLinux) {
    final home = env['HOME'] ?? (await getApplicationSupportDirectory()).path;
    return (
      configOverride ??
          p.join(
            env['XDG_CONFIG_HOME'] ?? p.join(home, '.config'),
            'gamehandler',
          ),
      dataOverride ??
          p.join(
            env['XDG_DATA_HOME'] ?? p.join(home, '.local', 'share'),
            'gamehandler',
          ),
    );
  }
  final support = (await getApplicationSupportDirectory()).path;
  return (configOverride ?? support, dataOverride ?? support);
}

/// Window geometry is UI state and is independent of Rust's game/settings schema.
class WindowPreferences extends WindowListener {
  WindowPreferences(this.file);
  final File file;
  Timer? debounce;
  bool closing = false;
  VoidCallback? beforeClose;
  Future<Size> load() async {
    try {
      final value =
          jsonDecode(await file.readAsString()) as Map<String, dynamic>;
      final width = (value['width'] as num).toDouble();
      final height = (value['height'] as num).toDouble();
      if (width.isFinite &&
          height.isFinite &&
          width >= 720 &&
          width <= 8192 &&
          height >= 480 &&
          height <= 8192) {
        return Size(width, height);
      }
    } catch (_) {
      /* Invalid geometry uses the visible default window. */
    }
    return const Size(1100, 760);
  }

  Future<void> save() async {
    try {
      final size = await windowManager.getSize();
      await file.parent.create(recursive: true);
      final temporary = File('${file.path}.tmp');
      await temporary.writeAsString(
        jsonEncode({'width': size.width, 'height': size.height}),
        flush: true,
      );
      await temporary.rename(file.path);
    } catch (error) {
      stderr.writeln('Could not save window size: $error');
    }
  }

  @override
  void onWindowResize() {
    if (closing) return;
    debounce?.cancel();
    debounce = Timer(const Duration(milliseconds: 500), save);
  }

  @override
  void onWindowClose() async {
    if (closing) return;
    closing = true;
    debounce?.cancel();
    beforeClose?.call();
    await save();
    await windowManager.destroy();
  }
}
