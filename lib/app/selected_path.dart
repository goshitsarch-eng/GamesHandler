import 'dart:convert';
import 'dart:io';

import 'package:dbus/dbus.dart';
import 'package:path/path.dart' as p;

/// Resolve a native chooser result against Flatpak's document mount.
/// Custom host runtime/cache directories can differ from the sandbox's path.
Future<String> selectedPath(String path) async {
  if (!Platform.isLinux ||
      !Platform.environment.containsKey('FLATPAK_ID') ||
      await FileSystemEntity.type(path) != FileSystemEntityType.notFound) {
    return path;
  }
  final runtime = Platform.environment['XDG_RUNTIME_DIR'];
  if (runtime == null || !p.isAbsolute(runtime)) return path;
  final client = DBusClient.session();
  try {
    final documents = DBusRemoteObject(
      client,
      name: 'org.freedesktop.portal.Documents',
      path: DBusObjectPath('/org/freedesktop/portal/documents'),
    );
    final reply = await documents
        .callMethod(
          'org.freedesktop.portal.Documents',
          'GetMountPoint',
          [],
          replySignature: DBusSignature('ay'),
        )
        .timeout(const Duration(seconds: 3));
    final hostMount = utf8.decode(
      reply.returnValues.single
          .asByteArray()
          .takeWhile((byte) => byte != 0)
          .toList(),
    );
    final resolved = documentPath(path, hostMount, runtime);
    if (await FileSystemEntity.type(resolved) !=
        FileSystemEntityType.notFound) {
      return resolved;
    }
  } catch (_) {
    // Rust will report an unavailable selection through its normal validation.
  } finally {
    await client.close();
  }
  return path;
}

/// Only translate documents inside the mount reported by the real portal.
String documentPath(String path, String hostMount, String runtime) {
  if (!p.posix.isAbsolute(hostMount) || !p.posix.isWithin(hostMount, path)) {
    return path;
  }
  return p.posix.join(runtime, 'doc', p.posix.relative(path, from: hostMount));
}
