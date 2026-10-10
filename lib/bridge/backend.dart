import 'dart:convert';
import 'dart:ffi';
import 'dart:io';
import 'dart:isolate';

import 'package:ffi/ffi.dart';
import 'package:path/path.dart' as p;

class BackendFailure implements Exception {
  const BackendFailure(this.kind, this.message);
  final String kind;
  final String message;
  @override
  String toString() => message;
}

/// Only serialized requests cross the isolate boundary. No widget handles,
/// native pointers or BuildContexts enter Rust. Each result has one owner.
class Backend {
  Backend({required this.config, required this.data, String? library})
    : library = library ?? defaultLibrary();
  final String config;
  final String data;
  final String library;

  static String defaultLibrary() {
    final override = Platform.environment['GAMEHANDLER_NATIVE_LIBRARY'];
    if (override != null && override.isNotEmpty) return override;
    final dir = p.dirname(Platform.resolvedExecutable);
    if (Platform.isWindows) return p.join(dir, 'gamehandler_desktop.dll');
    if (Platform.isMacOS) {
      return p.normalize(
        p.join(dir, '..', 'Frameworks', 'libgamehandler_desktop.dylib'),
      );
    }
    return p.join(dir, 'lib', 'libgamehandler_desktop.so');
  }

  Future<dynamic> call(
    String operation, [
    Map<String, dynamic>? arguments,
  ]) async {
    final request = jsonEncode({
      'context': {'config': config, 'data': data},
      'operation': operation,
      'arguments': ?arguments,
    });
    final path = library;
    final response = await Isolate.run(() => _request(path, request));
    final value = response;
    if (value['ok'] != true) {
      final error = value['error'] as Map<String, dynamic>;
      throw BackendFailure(error['kind'] as String, error['message'] as String);
    }
    return value['data'];
  }
}

Map<String, dynamic> _request(String path, String request) {
  final lib = DynamicLibrary.open(path);
  final version = lib.lookupFunction<Uint32 Function(), int Function()>(
    'gamehandler_abi_version',
  );
  if (version() != 1) {
    throw StateError('Incompatible GameHandler native library');
  }
  final invoke = lib
      .lookupFunction<
        Pointer<Utf8> Function(Pointer<Uint8>, Size),
        Pointer<Utf8> Function(Pointer<Uint8>, int)
      >('gamehandler_request');
  final free = lib
      .lookupFunction<
        Void Function(Pointer<Utf8>),
        void Function(Pointer<Utf8>)
      >('gamehandler_free');
  final input = request.toNativeUtf8();
  Pointer<Utf8> output = nullptr;
  try {
    output = invoke(input.cast(), utf8.encode(request).length);
    if (output == nullptr) throw StateError('The backend returned no response');
    return jsonDecode(output.toDartString()) as Map<String, dynamic>;
  } finally {
    if (output != nullptr) free(output);
    calloc.free(input);
  }
}
