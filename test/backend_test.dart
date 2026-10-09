import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:path/path.dart' as p;
import 'package:gamehandler/bridge/backend.dart';
import 'package:gamehandler/state/library_state.dart';

String libraryPath() =>
    Platform.environment['GAMEHANDLER_NATIVE_LIBRARY'] ??
    p.absolute(
      'rust',
      'target',
      'debug',
      Platform.isWindows
          ? 'gamehandler_desktop.dll'
          : Platform.isMacOS
          ? 'libgamehandler_desktop.dylib'
          : 'libgamehandler_desktop.so',
    );
void main() {
  late Directory root;
  late Backend backend;
  setUp(() async {
    root = await Directory.systemTemp.createTemp('gamehandler-bridge-');
    backend = Backend(
      config: p.join(root.path, 'config'),
      data: p.join(root.path, 'data'),
      library: libraryPath(),
    );
  });
  tearDown(() async {
    await root.delete(recursive: true);
  });

  test(
    'real bridge persists Unicode records and returns typed failures',
    () async {
      final exe = await File(p.join(root.path, '遊戯 executable'))
          .writeAsString('fixture');
      final game = Map<String, dynamic>.from(await backend.call('new_game'));
      game['name'] = 'Été 遊戯 🎮';
      game['exe_path'] = exe.path;
      await backend.call('save_game', {'game': game});
      final snapshot = await backend.call('snapshot', {
        'query': 'été',
        'category': 'All',
        'sort': 'name',
      });
      expect(snapshot['games'].single['name'], game['name']);
      expect(snapshot['games'].single['id'], game['id']);
      await expectLater(
        backend.call('launch', {'id': 'missing'}),
        throwsA(
          isA<BackendFailure>().having((e) => e.kind, 'kind', 'validation'),
        ),
      );
      await backend.call('remove_game', {'id': game['id']});
      expect(await exe.exists(), isTrue);
    },
  );

  test('queued preference changes preserve simultaneous user edits', () async {
    final state = LibraryState(backend);
    await state.refresh();
    await Future.wait([
      state.saveSettings('color_scheme', 'light'),
      state.saveSettings('view_mode', 'list'),
    ]);
    final value = await backend.call('snapshot', {});
    expect(value['settings']['color_scheme'], 'light');
    expect(value['settings']['view_mode'], 'list');
    state.dispose();
  });

  test(
    'empty values, concurrent calls and a large library survive the ABI',
    () async {
      final exe = await File(p.join(root.path, 'fixture'))
          .writeAsString('fixture');
      final template = Map<String, dynamic>.from(
        await backend.call('new_game'),
      );
      await Future.wait(
        List.generate(
          20,
          (i) => backend.call('save_game', {
            'game': {
              ...template,
              'id': 'fixture$i',
              'name': 'Game $i ${'界' * 250}',
              'exe_path': exe.path,
            },
          }),
        ),
      );
      final value = await backend.call('snapshot', {});
      expect(value['games'], hasLength(20));
      final filtered = await backend.call('snapshot', {'query': 'no match'});
      expect(filtered['games'], isEmpty);
    },
  );
}
