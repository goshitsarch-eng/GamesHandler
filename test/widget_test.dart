import 'dart:io';
import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:gamehandler/app/app.dart';
import 'package:gamehandler/bridge/backend.dart';
import 'package:gamehandler/state/library_state.dart';
import 'package:gamehandler/ui/game_editor.dart';

import 'backend_test.dart' show libraryPath;

void main() {
  testWidgets(
    'library renders real records and navigation has no overflow at desktop minimum',
    (tester) async {
      tester.view.physicalSize = const Size(720, 480);
      tester.view.devicePixelRatio = 1;
      final root = (await tester.runAsync(
        () => Directory.systemTemp.createTemp('gamehandler-widgets-'),
      ))!;
      final backend = Backend(
        config: '${root.path}/config',
        data: '${root.path}/data',
        library: libraryPath(),
      );
      final state = LibraryState(backend);
      await tester.runAsync(() async {
        final exe = await File('${root.path}/fixture').writeAsString('fixture');
        final game = Map<String, dynamic>.from(await backend.call('new_game'));
        game['name'] = 'Visible game';
        game['exe_path'] = exe.path;
        await backend.call('save_game', {'game': game});
        await state.refresh();
        await state.reloadCatalogue();
      });
      await tester.pumpWidget(GameHandlerApp(state: state, nativeMenus: false));
      await tester.pumpAndSettle();
      expect(find.text('Visible game'), findsOneWidget);
      expect(find.text('Play'), findsOneWidget);
      for (final label in [
        'Installers',
        'Plugins',
        'About',
        'Settings',
        'Library',
      ]) {
        await tester.ensureVisible(find.text(label).last);
        await tester.tap(find.text(label).last);
        await tester.pumpAndSettle();
        expect(tester.takeException(), isNull);
      }
      await tester.pumpWidget(const SizedBox());
      state.dispose();
      await tester.runAsync(() => root.delete(recursive: true));
      tester.view.resetPhysicalSize();
      tester.view.resetDevicePixelRatio();
    },
  );

  testWidgets(
    'editor exposes every saved launch toggle and cancel writes nothing',
    (tester) async {
      final root = (await tester.runAsync(
        () => Directory.systemTemp.createTemp('gamehandler-form-'),
      ))!;
      final state = LibraryState(
        Backend(
          config: '${root.path}/config',
          data: '${root.path}/data',
          library: libraryPath(),
        ),
      );
      late Map<String, dynamic> template;
      await tester.runAsync(() async {
        await state.refresh();
        await state.reloadCatalogue();
        template = Map<String, dynamic>.from(
          await state.backend.call('new_game'),
        );
      });
      await tester.pumpWidget(
        MaterialApp(
          home: GameEditor(state: state, initial: template),
        ),
      );
      expect(find.text('Executable or application bundle'), findsOneWidget);
      expect(find.text('Save'), findsOneWidget);
      expect(
        find.text('Compatibility and performance'),
        state.compatibility ? findsOneWidget : findsNothing,
      );
      await tester.pumpWidget(const SizedBox());
      state.dispose();
      expect(
        await tester.runAsync(
          () => File('${root.path}/config/games.json').exists(),
        ),
        isFalse,
      );
      await tester.runAsync(() => root.delete(recursive: true));
    },
  );
  testWidgets(
    'large real library stays virtualized and search finds offscreen records',
    (tester) async {
      final root = (await tester.runAsync(
        () => Directory.systemTemp.createTemp('gamehandler-large-'),
      ))!;
      final state = LibraryState(
        Backend(
          config: '${root.path}/config',
          data: '${root.path}/data',
          library: libraryPath(),
        ),
      );
      await tester.runAsync(() async {
        final config = await Directory('${root.path}/config').create();
        final executable = await File('${root.path}/fixture')
            .writeAsString('fixture');
        await File('${config.path}/games.json').writeAsString(
          jsonEncode(
            List.generate(
              1500,
              (i) => {
                'id': 'fixture-$i',
                'name': 'Fixture ${i.toString().padLeft(4, '0')}',
                'exe_path': executable.path,
                'kind': 'native',
                'category': 'Large library',
              },
            ),
          ),
        );
        await state.refresh();
      });
      await tester.pumpWidget(GameHandlerApp(state: state, nativeMenus: false));
      await tester.pumpAndSettle();
      expect(state.games, hasLength(1500));
      expect(find.text('Fixture 0000'), findsOneWidget);
      expect(find.text('Fixture 1499'), findsNothing);
      expect(find.textContaining('Fixture ').evaluate().length, lessThan(30));
      await tester.drag(find.byType(CustomScrollView), const Offset(0, -800));
      await tester.pumpAndSettle();
      expect(find.textContaining('Fixture ').evaluate().length, lessThan(30));
      state.query = 'Fixture 1499';
      await tester.runAsync(state.refresh);
      await tester.pumpAndSettle();
      expect(state.games, hasLength(1));
      expect(find.text('Fixture 1499'), findsOneWidget);
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox());
      state.dispose();
      await tester.runAsync(() => root.delete(recursive: true));
    },
  );
}
