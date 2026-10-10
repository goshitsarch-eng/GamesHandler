import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:window_manager/window_manager.dart';
import 'package:gamehandler/app/app.dart';
import 'package:gamehandler/bridge/backend.dart';
import 'package:gamehandler/state/library_state.dart';

/// Opt-in: opens real browser links and starts/cancels real runner downloads.
void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  testWidgets('live catalogue controls and every project link', (tester) async {
    final root = await Directory.systemTemp.createTemp(
      'gamehandler-network-audit-',
    );
    final state = LibraryState(
      Backend(config: '${root.path}/config', data: '${root.path}/data'),
    );
    await state.initialize();
    await windowManager.ensureInitialized();
    await windowManager.setSize(const Size(1100, 760));
    await windowManager.show();
    await tester.pumpWidget(GameHandlerApp(state: state));
    Future<void> settle() async {
      for (var i = 0; i < 5; i++) {
        await tester.pump(const Duration(milliseconds: 100));
      }
      expect(tester.takeException(), isNull);
    }

    Future<void> click(Finder target) async {
      if (target.evaluate().isEmpty) {
        await tester.scrollUntilVisible(
          target,
          250,
          scrollable: find.byType(Scrollable).last,
          maxScrolls: 80,
        );
      }
      await tester.ensureVisible(target.first);
      await tester.pump();
      await tester.tap(target.first);
      await settle();
    }

    Future<void> page(String name) => click(
      find.descendant(
        of: find.byType(NavigationRail),
        matching: find.text(name),
      ),
    );
    Future<void> link(Finder target, String url) async {
      await click(target);
      debugPrint('AUDIT LINK ACTIVATED: $url');
      // Optional QA desktop: close the opened browser tab while retaining its base tab.
      final display = Platform.environment['GAMEHANDLER_AUDIT_BROWSER_DISPLAY'];
      if (display != null) {
        await Future<void>.delayed(const Duration(seconds: 2));
        final browser = await Process.run(
          'xdotool',
          [
            'search',
            '--onlyvisible',
            '--class',
            'chromium',
            'windowactivate',
            '--sync',
            'key',
            '--clearmodifiers',
            'ctrl+w',
          ],
          environment: {'DISPLAY': display},
        );
        expect(
          browser.exitCode,
          0,
          reason: 'The browser handoff must leave a real window to activate',
        );
      }
    }

    await settle();
    if (Platform.environment['GAMEHANDLER_AUDIT_RUNNERS_ONLY'] != '1') {
      await page('About');
      for (final name in ['Flutter', 'Dart', 'Rust']) {
        final tile = find.ancestor(
          of: find.text('$name · $name contributors'),
          matching: find.byType(ListTile),
        );
        await link(
          find.descendant(of: tile, matching: find.text('Visit')),
          name,
        );
      }
      for (final section in state.catalogue['credits']) {
        for (final entry in section['entries']) {
          final tile = find.ancestor(
            of: find.text('${entry['name']} · ${entry['authors']}'),
            matching: find.byType(ListTile),
          );
          await link(
            find.descendant(of: tile, matching: find.text('Visit')),
            entry['url'],
          );
        }
      }
    }
    await page('Runners');
    for (final family in state.catalogue['families']) {
      // Return to the top before switching family; dropdowns outside the viewport remain reachable.
      await tester.drag(find.byType(ListView).first, const Offset(0, 3000));
      await settle();
      await click(find.byType(DropdownButton<String>).first);
      await click(find.text(family['name']).last);
      for (
        var i = 0;
        i < 160 && find.byType(LinearProgressIndicator).evaluate().isNotEmpty;
        i++
      ) {
        await tester.pump(const Duration(milliseconds: 250));
      }
      await settle();
      if (Platform.environment['GAMEHANDLER_AUDIT_RUNNERS_ONLY'] != '1') {
        await link(find.text('Visit project'), family['url']);
      }
      if (find.text('Install').evaluate().isNotEmpty) {
        await click(find.text('Install').first);
        expect(state.jobs, isNotEmpty);
        final id = state.jobs.single['id'];
        if (state.jobs.single['status'] == 'running') {
          await click(find.text('Cancel'));
          for (
            var i = 0;
            i < 240 && state.jobs.single['status'] == 'running';
            i++
          ) {
            await tester.pump(const Duration(milliseconds: 250));
            await state.pollJobs();
          }
        }
        expect(state.jobs.single['status'], isNot('running'));
        debugPrint(
          'AUDIT RUNNER: ${family['id']} job $id ${state.jobs.single['status']}',
        );
        await click(find.text('Dismiss'));
      } else {
        debugPrint('AUDIT RUNNER NO INSTALLABLE RELEASE: ${family['id']}');
        debugPrint(
          'RUNNER PAGE: ${tester.widgetList<Text>(find.byType(Text)).map((t) => t.data).toList()}',
        );
        final actual = await state.backend.call('releases', {
          'family': family['id'],
        }) as List;
        expect(
          actual,
          isEmpty,
          reason: 'Available releases must expose Install controls',
        );
      }
      await tester.drag(find.byType(ListView).first, const Offset(0, 3000));
      await settle();
      await click(find.text('Refresh'));
      for (
        var i = 0;
        i < 160 && find.byType(LinearProgressIndicator).evaluate().isNotEmpty;
        i++
      ) {
        await tester.pump(const Duration(milliseconds: 250));
      }
      await settle();
    }
    for (final size in [
      const Size(720, 480),
      const Size(1200, 480),
      const Size(720, 850),
      const Size(1100, 760),
    ]) {
      await windowManager.setSize(size);
      await settle();
    }
    tester.platformDispatcher.textScaleFactorTestValue = 2;
    await windowManager.setSize(const Size(720, 480));
    await settle();
    tester.platformDispatcher.clearTextScaleFactorTestValue();
    debugPrint(
      'AUDIT PASS: live runner page at minimum/wide/tall/normal and 200% text',
    );
    await tester.pumpWidget(const SizedBox());
    state.dispose();
    await root.delete(recursive: true);
  }, skip: Platform.environment['GAMEHANDLER_NETWORK_AUDIT'] != '1');
}
