import 'dart:io';
import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:window_manager/window_manager.dart';
import 'package:gamehandler/app/app.dart';
import 'package:gamehandler/bridge/backend.dart';
import 'package:gamehandler/models/game.dart';
import 'package:gamehandler/state/library_state.dart';
import 'package:gamehandler/ui/game_editor.dart';
import 'package:gamehandler/ui/catalogue.dart';
import 'package:gamehandler/ui/settings.dart';
import 'package:gamehandler/ui/library.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  testWidgets('real desktop exhaustive local controls and persistence', (
    tester,
  ) async {
    // Test bindings suppress ServicesBinding's bundled license registration.
    // Load the real build's notices using the production bundle format.
    LicenseRegistry.addLicense(() async* {
      final bytes = await rootBundle.load('NOTICES.Z');
      final notices = utf8.decode(gzip.decode(bytes.buffer.asUint8List()));
      for (final notice in notices.split('\n${'-' * 80}\n')) {
        final split = notice.indexOf('\n\n');
        if (split >= 0) {
          yield LicenseEntryWithLineBreaks(
            notice.substring(0, split).split('\n'),
            notice.substring(split + 2),
          );
        }
      }
    });
    tester.testTextInput.register();
    addTearDown(tester.testTextInput.unregister);
    final root = await Directory.systemTemp.createTemp('gamehandler-ui-audit-');
    final state = LibraryState(
      Backend(config: '${root.path}/config', data: '${root.path}/data'),
    );
    await state.initialize();
    await windowManager.ensureInitialized();
    await windowManager.setSize(const Size(1100, 760));
    await windowManager.show();
    await tester.pumpWidget(GameHandlerApp(state: state));
    Future<void> settle() async {
      for (var i = 0; i < 8; i++) {
        await tester.pump(const Duration(milliseconds: 100));
      }
      expect(tester.takeException(), isNull);
    }

    Finder contentScroll() {
      for (final type in [
        GameEditor,
        LicensePage,
        AboutPage,
        PluginsPage,
        InstallersPage,
        SettingsPage,
        LibraryPage,
      ]) {
        final page = find.byType(type);
        if (page.evaluate().isNotEmpty) {
          return find
              .descendant(of: page, matching: find.byType(Scrollable))
              .first;
        }
      }
      throw StateError('No active scrollable audit page');
    }

    Future<void> click(Finder finder) async {
      if (finder.evaluate().isEmpty) {
        await tester.scrollUntilVisible(
          finder,
          200,
          scrollable: contentScroll(),
          maxScrolls: 60,
        );
      }
      await tester.ensureVisible(finder.first);
      await tester.pump();
      await tester.tap(finder.first);
      await settle();
    }

    Future<void> text(String label) => click(find.text(label));
    Future<void> page(String label) => click(
      find.descendant(
        of: find.byType(NavigationRail),
        matching: find.text(label),
      ),
    );
    Finder field(String label) => find.byWidgetPredicate(
      (w) => w is TextField && w.decoration?.labelText == label,
    );
    Future<void> input(String label, String value) async {
      final f = field(label);
      if (f.evaluate().isEmpty) {
        await tester.scrollUntilVisible(
          f,
          200,
          scrollable: find.byType(GameEditor).evaluate().isNotEmpty
              ? find
                    .descendant(
                      of: find.byType(GameEditor),
                      matching: find.byType(Scrollable),
                    )
                    .last
              : find
                    .byElementPredicate(
                      (e) =>
                          e.widget is Scrollable &&
                          e.findAncestorWidgetOfExactType<NavigationRail>() ==
                              null,
                    )
                    .first,
          maxScrolls: 60,
        );
      }
      await tester.ensureVisible(f);
      await tester.enterText(f, value);
      expect(tester.widget<TextField>(f).controller!.text, value);
      await settle();
    }

    Future<void> select(String label, String value) async {
      await click(
        find
            .ancestor(
              of: find.text(label),
              matching: find.byType(DropdownButtonFormField<String>),
            )
            .first,
      );
      await click(find.text(value).last);
    }

    Future<void> escape() async {
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await settle();
    }

    void evidence(String value) => debugPrint('AUDIT PASS: $value');
    final licensesOnly =
        Platform.environment['GAMEHANDLER_AUDIT_LICENSES_ONLY'] == '1';
    if (!licensesOnly) {
      await settle();
      expect(find.text('No games yet'), findsOneWidget);
      await click(find.widgetWithText(FilledButton, 'Add game').last);
      await text('Save');
      expect(find.text('Enter a game name'), findsOneWidget);
      await input('Name', '   ');
      await text('Save');
      expect(find.text('Enter a game name'), findsOneWidget);
      await input('Name', 'QA café 遊戯 "quoted"');
      await input('Executable or application bundle', '/missing/QA game');
      await text('Save');
      expect(find.byType(GameEditor), findsOneWidget);
      await input('Executable or application bundle', root.path);
      await text('Save');
      expect(find.byType(GameEditor), findsOneWidget);
      await input('Executable or application bundle', '/bin/true');
      await select('Type', 'Linux native');
      await input('Category', 'QA category');
      await input('Arguments', '--example "value with spaces"');
      await input('Working directory', root.path);
      await input('Additional application', '');
      await input('Cover image', '/missing/cover.png');
      await input(
        'Environment (NAME=value, one per line)',
        'QA_VALUE=café\nSECOND=value with spaces',
      );
      await text('Save');
      expect(find.byType(GameEditor), findsOneWidget);
      await input('Cover image', '');
      await text('Save');
      if (find.byType(GameEditor).evaluate().isNotEmpty) {
        debugPrint(
          'EDITOR CONTENT: ${tester.widgetList<Text>(find.descendant(of: find.byType(GameEditor), matching: find.byType(Text))).map((t) => t.data).toList()}',
        );
      }
      expect(find.byType(GameEditor), findsNothing);
      expect(state.games.single.name, 'QA café 遊戯 "quoted"');
      evidence(
        'empty and whitespace validation, missing executable, Unicode fields, native type, save, missing cover validation',
      );

      await click(find.byTooltip('Actions for ${state.games.single.name}'));
      await text('Edit');
      await input('Name', 'Cancelled change');
      await escape();
      expect(state.games.single.name, 'QA café 遊戯 "quoted"');
      await click(find.byTooltip('Actions for ${state.games.single.name}'));
      await text('Edit');
      for (final type in [
        'Native application',
        'macOS native',
        'Windows game',
      ]) {
        await select('Type', type);
      }
      await text('Compatibility and performance');
      for (final entry in launchToggles.entries) {
        await text(entry.value);
        final tile = tester.widget<SwitchListTile>(
          find.widgetWithText(SwitchListTile, entry.value),
        );
        final changed = tile.value;
        await text(entry.value);
        expect(
          tester
              .widget<SwitchListTile>(
                find.widgetWithText(SwitchListTile, entry.value),
              )
              .value,
          !changed,
        );
        evidence('editor toggle ${entry.value} on/off');
      }
      await text('Virtual desktop');
      await input('Virtual desktop size, e.g. 1920x1080', '1280x720');
      await text('Cancel');
      evidence(
        'all game types and all 15 launch toggles; virtual desktop input; cancel keeps original',
      );

      await tester.enterText(find.byType(TextField).first, 'no-match-遊戯');
      await settle();
      expect(find.text('No games match your filters'), findsOneWidget);
      await page('Settings');
      await page('Library');
      expect(
        tester.widget<TextField>(find.byType(TextField).first).controller!.text,
        'no-match-遊戯',
      );
      await text('Clear filters');
      await settle();
      expect(state.games, hasLength(1));
      evidence(
        'search, navigation retains visible query, clear filters restores records',
      );
      await click(find.byTooltip('List view'));
      expect(state.settings['view_mode'], 'list');
      await click(find.byTooltip('Grid view'));
      expect(state.settings['view_mode'], 'grid');
      for (final sort in ['Recently played', 'Recently added', 'Name']) {
        await click(find.byType(DropdownButton<String>).last);
        await click(find.text(sort).last);
      }
      await click(find.byType(DropdownButton<String>).first);
      await click(find.text('QA category').last);
      expect(state.category, 'QA category');
      await click(find.byType(DropdownButton<String>).first);
      await click(find.text('All').last);
      await text('Play');
      expect(state.games.single.fields['last_played'], isNotNull);
      evidence('grid/list, three sorts, category selection and native Play');

      await page('Settings');
      for (final theme in ['Light', 'Dark', 'Follow system']) {
        await select('Theme', theme);
      }
      for (final view in ['List', 'Grid']) {
        await select('Library layout', view);
      }
      for (final sort in ['Recently played', 'Recently added', 'Name']) {
        await select('Default sort', sort);
      }
      await text('Minimize window after launching a game');
      expect(state.settings['close_on_launch'], true);
      await text('Minimize window after launching a game');
      for (final entry in launchToggles.entries.where(
        (e) => state.settings.containsKey('default_${e.key}'),
      )) {
        final label = 'Enable ${entry.value} by default';
        final original = state.settings['default_${entry.key}'];
        await text(label);
        expect(state.settings['default_${entry.key}'], !original);
        await text(label);
        expect(state.settings['default_${entry.key}'], original);
        evidence('persisted settings toggle ${entry.key} on/off');
      }
      evidence(
        'theme/layout/sort choices, hide-on-launch and every available default toggle',
      );

      for (final size in [
        const Size(720, 480),
        const Size(1200, 480),
        const Size(720, 850),
        const Size(1100, 760),
      ]) {
        await windowManager.setSize(size);
        await settle();
        for (final name in [
          'Library',
          'Installers',
          'Plugins',
          'About',
          'Settings',
        ]) {
          await page(name);
          evidence('$name at ${size.width}x${size.height}');
        }
      }
      await page('Plugins');
      for (final plugin in state.catalogue['plugins']) {
        if (plugin['status'] == 'installed') continue;
        final tile = find.ancestor(
          of: find.text('${plugin['name']} · ${plugin['status']}'),
          matching: find.byType(ListTile),
        );
        await click(
          find.descendant(
            of: tile,
            matching: find.text('Install instructions'),
          ),
        );
        expect(find.text('Install ${plugin['name']}'), findsOneWidget);
        await text('Copy command');
        final clipboard = await Clipboard.getData('text/plain');
        final expected = await state.backend.call('plugin_command', {
          'id': plugin['id'],
        });
        expect(clipboard?.text, expected['display']);
        await text('Close');
        evidence('plugin ${plugin['id']} instructions, real clipboard, close');
      }
      await text('Refresh detection');
      await page('Installers');
      final installerSearch = find.byType(TextField).first;
      await tester.enterText(installerSearch, 'no-such-installer');
      await settle();
      expect(find.text('Install'), findsNothing);
      await tester.enterText(installerSearch, '');
      await settle();
      for (final category in <String>{
        'All',
        ...state.catalogue['installers'].map<String>(
          (dynamic i) => i['category'] as String,
        ),
      }) {
        await click(find.byType(DropdownButton<String>).first);
        await click(find.text(category).last);
        evidence('installer category $category');
      }
      await click(find.byType(DropdownButton<String>).first);
      await click(find.text('All').last);
      // Retire the earlier native-launch job; job IDs are not chronological.
      await state.pollJobs();
      while (find.text('Dismiss').evaluate().isNotEmpty) {
        await text('Dismiss');
      }
      for (final installer in state.catalogue['installers']) {
        final tile = find.ancestor(
          of: find.text(installer['name']),
          matching: find.byType(ListTile),
        );
        await click(find.descendant(of: tile, matching: find.text('Install')));
        for (
          var i = 0;
          i < 20 &&
              (state.jobs.isEmpty || state.jobs.last['status'] == 'running');
          i++
        ) {
          await tester.pump(const Duration(milliseconds: 250));
          await state.pollJobs();
        }
        expect(state.jobs.last['status'], 'failed');
        await text('Dismiss');
        for (var i = 0; i < 20 && state.jobs.isNotEmpty; i++) {
          await tester.pump(const Duration(milliseconds: 250));
          await state.pollJobs();
        }
        expect(state.jobs, isEmpty);
        await settle();
        evidence(
          'installer ${installer['id']} actual unavailable-runner error and dismiss',
        );
      }
    }
    await page('About');
    await windowManager.setSize(const Size(720, 480));
    await settle();
    await text('Open-source licenses');
    expect(find.byType(LicensePage), findsOneWidget);
    await settle();
    final packages = <String>{};
    await for (final entry in LicenseRegistry.licenses) {
      packages.addAll(entry.packages);
    }
    expect(packages, isNotEmpty);
    final packageOrder = packages.skip(1).toList()
      ..sort((a, b) => a.toLowerCase().compareTo(b.toLowerCase()));
    packageOrder.insert(0, packages.first);
    for (final package in packageOrder) {
      final tile = find.descendant(
        of: find.byType(ListTile),
        matching: find.text(package),
      );
      if (tile.evaluate().isEmpty) {
        await tester.scrollUntilVisible(
          tile,
          200,
          scrollable: contentScroll(),
          maxScrolls: 60,
        );
      }
      await tester.ensureVisible(tile);
      await tester.pump();
      await tester.tap(tile);
      await tester.pumpAndSettle(const Duration(milliseconds: 50));
      expect(tester.takeException(), isNull);
      await tester.pageBack();
      await tester.pumpAndSettle(const Duration(milliseconds: 50));
      expect(find.byType(LicensePage), findsOneWidget);
      evidence('license package $package detail and back');
    }
    await tester.pageBack();
    await settle();
    evidence('licenses page and back');
    if (licensesOnly) {
      await tester.pumpWidget(const SizedBox());
      state.dispose();
      await root.delete(recursive: true);
      return;
    }
    tester.platformDispatcher.textScaleFactorTestValue = 2;
    await windowManager.setSize(const Size(720, 480));
    await settle();
    for (final name in [
      'Library',
      'Installers',
      'Plugins',
      'About',
      'Settings',
    ]) {
      await page(name);
      evidence('$name at 720x480, text scale 200%');
    }
    tester.platformDispatcher.clearTextScaleFactorTestValue();
    await windowManager.setSize(const Size(1100, 760));
    await settle();
    await page('Library');
    await click(find.byTooltip('Actions for ${state.games.single.name}'));
    await text('Remove from library');
    await text('Cancel');
    expect(state.games, hasLength(1));
    await click(find.byTooltip('Actions for ${state.games.single.name}'));
    await text('Remove from library');
    state.category = 'QA category';
    await state.refresh();
    await text('Remove');
    expect(state.games, isEmpty);
    expect(state.category, 'All');
    evidence('removed last category resets the active filter');
    evidence('remove cancellation and confirmation');
    final reopened = LibraryState(
      Backend(config: '${root.path}/config', data: '${root.path}/data'),
    );
    await reopened.initialize();
    expect(reopened.games, isEmpty);
    expect(reopened.settings, state.settings);
    reopened.dispose();
    final libraryFile = File('${root.path}/config/games.json');
    final goodLibrary = await libraryFile.readAsBytes();
    await libraryFile.writeAsString('{broken library');
    await state.refresh();
    await settle();
    expect(state.error, isNotNull);
    expect(await libraryFile.readAsString(), '{broken library');
    expect(find.text('Retry'), findsWidgets);
    await libraryFile.writeAsBytes(goodLibrary);
    await text('Retry');
    expect(state.error, isNull);
    evidence(
      'corrupt library shows error, preserves bytes, Retry recovers after repair',
    );
    await tester.pumpWidget(const SizedBox());
    state.dispose();
    await root.delete(recursive: true);
    evidence('reopened real Rust store preserves settings and deletion');
  });
}
