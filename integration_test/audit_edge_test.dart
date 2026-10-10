import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:window_manager/window_manager.dart';
import 'package:gamehandler/app/app.dart';
import 'package:gamehandler/bridge/backend.dart';
import 'package:gamehandler/state/library_state.dart';
import 'package:gamehandler/ui/game_editor.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  testWidgets('real desktop unavailable runner, scaled editor and restoration', (
    tester,
  ) async {
    tester.testTextInput.register();
    addTearDown(tester.testTextInput.unregister);
    final root = await Directory.systemTemp.createTemp(
      'gamehandler-edge-audit-',
    );
    final state = LibraryState(
      Backend(config: '${root.path}/config', data: '${root.path}/data'),
    );
    await state.initialize();
    await state.saveSettings('default_runner', 'wine-removed');
    await windowManager.ensureInitialized();
    await windowManager.setSize(const Size(720, 480));
    await windowManager.show();
    await tester.pumpWidget(GameHandlerApp(state: state));
    Future<void> settle() async {
      for (var i = 0; i < 6; i++) {
        await tester.pump(const Duration(milliseconds: 100));
      }
      expect(tester.takeException(), isNull);
    }

    Future<void> click(Finder target) async {
      if (target.evaluate().isEmpty) {
        await tester.scrollUntilVisible(
          target,
          180,
          scrollable: find.byType(Scrollable).last,
          maxScrolls: 60,
        );
      }
      await tester.ensureVisible(target.first);
      await tester.pump();
      await tester.tap(target.first);
      await settle();
    }

    await settle();
    await click(find.text('Settings').last);
    await click(find.text('wine-removed (not installed)'));
    await click(find.text('System Wine').last);
    expect(state.settings['default_runner'], 'wine-system');
    await state.saveSettings('default_runner', 'wine-removed');
    await click(find.byTooltip('Add game'));
    await click(
      find.descendant(
        of: find.byType(GameEditor),
        matching: find.text('wine-removed (not installed)'),
      ),
    );
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await settle();
    expect(find.byType(GameEditor), findsOneWidget);
    tester.platformDispatcher.textScaleFactorTestValue = 2;
    await settle();
    await click(find.text('Compatibility and performance'));
    await click(find.text('Virtual desktop'));
    await settle();
    await click(find.text('Save'));
    expect(find.text('Enter a game name'), findsOneWidget);
    await click(find.text('Cancel'));
    tester.platformDispatcher.clearTextScaleFactorTestValue();
    await windowManager.setSize(const Size(1100, 760));
    await settle();
    final game = Map<String, dynamic>.from(
      await state.backend.call('new_game'),
    );
    game.addAll({
      'name': 'Minimize QA',
      'kind': 'linux',
      'exe_path': '/bin/true',
    });
    await state.saveGame(game);
    await state.saveSettings('close_on_launch', true);
    await click(find.text('Library').last);
    await tester.tap(find.text('Play'));
    // A minimized native window stops supplying frames; restore outside pump.
    await tester.runAsync(() async {
      await Future<void>.delayed(const Duration(seconds: 2));
      expect(await windowManager.isMinimized(), true);
      await windowManager.restore();
      await windowManager.focus();
    });
    await settle();
    expect(await windowManager.isMinimized(), false);
    expect(find.text('Minimize QA'), findsOneWidget);
    await click(find.byTooltip('Actions for Minimize QA'));
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await settle();
    await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.comma);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
    await settle();
    expect(find.text('Appearance'), findsOneWidget);
    debugPrint(
      'AUDIT PASS: missing default and editor runner, replacement saved, 200% editor at minimum, validation/cancel, actual minimize/restore, shortcut after menu',
    );
    final failing = File('${root.path}/failure.sh');
    await failing.writeAsString(
      '#!/bin/sh\nsleep 1\nprintf "%s" "${'Long failure detail '.padRight(300, 'x')}" >&2\nexit 7\n',
    );
    final chmod = await Process.run('chmod', ['+x', failing.path]);
    expect(chmod.exitCode, 0);
    final failedGame = Map<String, dynamic>.from(
      await state.backend.call('new_game'),
    );
    failedGame.addAll({
      'name': 'ZZ failure',
      'kind': 'linux',
      'exe_path': failing.path,
    });
    await state.saveGame(failedGame);
    await state.saveSettings('close_on_launch', false);
    await click(find.text('Library').last);
    final card = find.ancestor(
      of: find.text('ZZ failure'),
      matching: find.byType(Card),
    );
    await click(find.descendant(of: card, matching: find.text('Play')));
    for (var i = 0; i < 20 && state.error == null; i++) {
      await tester.pump(const Duration(milliseconds: 250));
      await state.pollJobs();
    }
    expect(state.error, isNotNull);
    await windowManager.setSize(const Size(720, 480));
    tester.platformDispatcher.textScaleFactorTestValue = 2;
    await settle();
    await click(find.text('Retry'));
    expect(state.error, isNull);
    tester.platformDispatcher.clearTextScaleFactorTestValue();
    debugPrint(
      'AUDIT PASS: actual long launch failure and Retry at minimum with 200% text',
    );
    await tester.pumpWidget(const SizedBox());
    state.dispose();
    await root.delete(recursive: true);
  });
}
