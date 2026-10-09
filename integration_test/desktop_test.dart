import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:gamehandler/app/app.dart';
import 'package:gamehandler/bridge/backend.dart';
import 'package:gamehandler/state/library_state.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  testWidgets('desktop library uses the packaged Rust backend', (tester) async {
    final root = await Directory.systemTemp.createTemp('gamehandler-desktop-');
    final backend = Backend(
      config: '${root.path}/config',
      data: '${root.path}/data',
    );
    final state = LibraryState(backend);
    final exe = await File('${root.path}/Unicode 遊戯').writeAsString('fixture');
    final game = Map<String, dynamic>.from(await backend.call('new_game'));
    game['name'] = 'Desktop integration 遊戯';
    game['exe_path'] = exe.path;
    await state.saveGame(game);
    await state.reloadCatalogue();
    await tester.pumpWidget(GameHandlerApp(state: state));
    await tester.pumpAndSettle();
    expect(find.text('Desktop integration 遊戯'), findsOneWidget);
    await state.saveSettings('color_scheme', 'light');
    await tester.pumpAndSettle();
    expect(
      Theme.of(tester.element(find.text('Desktop integration 遊戯'))).brightness,
      Brightness.light,
    );
    await state.remove(game['id']);
    await tester.pumpAndSettle();
    expect(find.text('No games yet'), findsOneWidget);
    await tester.pumpWidget(const SizedBox());
    state.dispose();
    await root.delete(recursive: true);
  });
}
