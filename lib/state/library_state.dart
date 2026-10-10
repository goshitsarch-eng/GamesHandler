import 'dart:async';

import 'package:flutter/foundation.dart';

import '../bridge/backend.dart';
import '../models/game.dart';

/// Presentation state only. Persistent records are fetched from Rust.
class LibraryState extends ChangeNotifier {
  LibraryState(this.backend);
  final Backend backend;
  List<GameDto> games = [];
  List<String> categories = [];
  Map<String, dynamic> settings = {};
  Map<String, dynamic> catalogue = {};
  List<Map<String, dynamic>> jobs = [];
  String query = '', category = 'All', sort = 'name', platform = '';
  String version = '';
  String? error;
  bool loading = true, compatibility = false;
  int _request = 0;
  int launchFailureVersion = 0;
  int artworkRevision = 0;

  void invalidateArtwork() {
    artworkRevision++;
  }

  bool _disposed = false;
  bool _polling = false;
  Timer? _timer;
  Timer? _debounce;
  Future<void> _settingsQueue = Future<void>.value();

  Future<void> initialize() async {
    await refresh();
    final preferred = settings['sort_mode'];
    if (preferred is String &&
        const ['name', 'recent', 'added'].contains(preferred) &&
        preferred != sort) {
      sort = preferred;
      await refresh();
    }
    await reloadCatalogue();
    _timer = Timer.periodic(const Duration(seconds: 1), (_) => pollJobs());
  }

  Future<void> refresh() async {
    if (_disposed) return;
    final token = ++_request;
    loading = true;
    notifyListeners();
    try {
      final value = await backend.call('snapshot', {
        'query': query,
        'category': category,
        'sort': sort,
      }) as Map<String, dynamic>;
      if (_disposed || token != _request) return;
      games = (value['games'] as List)
          .map((g) => GameDto(Map<String, dynamic>.from(g)))
          .toList();
      categories = List<String>.from(value['categories']);
      if (category != 'All' && !categories.contains(category)) {
        category = 'All';
        await refresh();
        return;
      }
      settings = Map<String, dynamic>.from(value['settings']);
      platform = value['platform'];
      version = value['version'];
      compatibility = value['compatibility'];
      error = null;
    } catch (e) {
      if (!_disposed && token == _request) error = e.toString();
    } finally {
      if (!_disposed && token == _request) {
        loading = false;
        notifyListeners();
      }
    }
  }

  Future<void> reloadCatalogue() async {
    try {
      final result = await backend.call('catalogue');
      if (!_disposed) catalogue = Map<String, dynamic>.from(result);
    } catch (e) {
      if (!_disposed) error = e.toString();
    }
    if (!_disposed) notifyListeners();
  }

  void search(String value) {
    query = value;
    _debounce?.cancel();
    _debounce = Timer(const Duration(milliseconds: 200), refresh);
  }

  Future<void> saveSettings(String key, dynamic value) {
    final operation = _settingsQueue.then((_) async {
      final next = {...settings, key: value};
      await backend.call('save_settings', {'settings': next});
      settings = next;
      if (key == 'sort_mode') sort = value as String;
      await refresh();
    });
    // The returned operation reports errors to the caller; the queue stays usable.
    _settingsQueue = operation.then(
      (_) {},
      onError: (Object e, StackTrace s) {},
    );
    return operation;
  }

  Future<void> saveGame(Map<String, dynamic> game) async {
    await backend.call('save_game', {'game': game});
    invalidateArtwork();
    await refresh();
  }

  Future<void> remove(String id) async {
    await backend.call('remove_game', {'id': id});
    await refresh();
  }

  Future<void> launch(String id) async {
    await backend.call('launch', {'id': id});
    await refresh();
  }

  Future<void> startJob(String kind, Map<String, dynamic> payload) async {
    await backend.call('start_job', {'kind': kind, 'payload': payload});
    await pollJobs();
  }

  Future<void> pollJobs() async {
    if (_disposed || _polling) return;
    _polling = true;
    try {
      final result = await backend.call('jobs') as List;
      if (_disposed) return;
      final next = result.map((j) => Map<String, dynamic>.from(j)).toList();
      final changed = next.any(
        (j) =>
            j['status'] != 'running' &&
            !jobs.any(
              (old) => old['id'] == j['id'] && old['status'] == j['status'],
            ),
      );
      final failures = next
          .where(
            (j) =>
                j['kind'] == 'launch' &&
                j['status'] == 'failed' &&
                !jobs.any(
                  (old) => old['id'] == j['id'] && old['status'] == 'failed',
                ),
          )
          .toList();
      jobs = next;
      if (changed) {
        if (next.any(
          (job) => job['kind'] == 'cover' && job['status'] == 'complete',
        )) {
          invalidateArtwork();
        }
        await refresh();
        await reloadCatalogue();
      }
      if (failures.isNotEmpty) {
        error = failures.last['error']['message'] as String;
        launchFailureVersion++;
      }
      if (!_disposed) notifyListeners();
    } catch (e) {
      if (!_disposed) {
        error = e.toString();
        notifyListeners();
      }
    } finally {
      _polling = false;
    }
  }

  @override
  void dispose() {
    _disposed = true;
    _timer?.cancel();
    _debounce?.cancel();
    super.dispose();
  }
}
