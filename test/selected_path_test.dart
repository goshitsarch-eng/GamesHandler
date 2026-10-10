import 'package:flutter_test/flutter_test.dart';
import 'package:gamehandler/app/selected_path.dart';

void main() {
  test(
    'portal documents map to the sandbox runtime without changing other paths',
    () {
      expect(
        documentPath(
          '/host/cache/doc/abcd/測試 game.sh',
          '/host/cache/doc',
          '/run/user/1000',
        ),
        '/run/user/1000/doc/abcd/測試 game.sh',
      );
      for (final path in [
        '/host/cache/document/game.sh',
        '/games/game.sh',
        '/host/cache/doc/../private.sh',
      ]) {
        expect(documentPath(path, '/host/cache/doc', '/run/user/1000'), path);
      }
    },
  );
}
