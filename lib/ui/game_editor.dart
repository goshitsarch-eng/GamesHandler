import 'package:flutter/material.dart';
import 'package:file_selector/file_selector.dart';

import '../models/game.dart';
import '../state/library_state.dart';

class GameEditor extends StatefulWidget {
  const GameEditor({super.key, required this.state, required this.initial});
  final LibraryState state;
  final Map<String, dynamic> initial;
  @override
  State<GameEditor> createState() => _GameEditorState();
}

class _GameEditorState extends State<GameEditor> {
  late Map<String, dynamic> value;
  final form = GlobalKey<FormState>();
  bool saving = false;
  String? error;
  final controllers = <String, TextEditingController>{};
  @override
  void initState() {
    super.initState();
    value = {...widget.initial};
  }

  @override
  void dispose() {
    for (final c in controllers.values) {
      c.dispose();
    }
    super.dispose();
  }

  TextEditingController controller(String key) => controllers.putIfAbsent(
    key,
    () => TextEditingController(text: '${value[key] ?? ''}'),
  );
  Future<void> choose(String key, {bool directory = false}) async {
    final path = directory
        ? await getDirectoryPath()
        : (await openFile())?.path;
    if (path != null && mounted) {
      setState(() {
        value[key] = path;
        controller(key).text = path;
      });
    }
  }

  Widget field(
    String key,
    String label, {
    bool browse = false,
    bool directory = false,
    int lines = 1,
  }) => Padding(
    padding: const EdgeInsets.only(bottom: 12),
    child: TextFormField(
      controller: controller(key),
      autofocus: key == 'name',
      maxLines: lines,
      decoration: InputDecoration(
        labelText: label,
        border: const OutlineInputBorder(),
        suffixIcon: browse
            ? IconButton(
                tooltip: 'Choose $label',
                onPressed: () => choose(key, directory: directory),
                icon: const Icon(Icons.folder_open),
              )
            : null,
      ),
      onChanged: (v) => value[key] = v,
    ),
  );
  Future<void> save() async {
    setState(() {
      saving = true;
      error = null;
    });
    try {
      await widget.state.saveGame(value);
      if (mounted) Navigator.pop(context);
    } catch (e) {
      if (mounted) setState(() => error = e.toString());
    } finally {
      if (mounted) setState(() => saving = false);
    }
  }

  @override
  Widget build(BuildContext context) => AlertDialog(
    title: Text(widget.initial['name'] == '' ? 'Add game' : 'Edit game'),
    content: SizedBox(
      width: 640,
      child: SingleChildScrollView(
        child: Form(
          key: form,
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              if (error != null)
                Padding(
                  padding: const EdgeInsets.only(bottom: 12),
                  child: Text(
                    error!,
                    style: TextStyle(
                      color: Theme.of(context).colorScheme.error,
                    ),
                  ),
                ),
              field('name', 'Name'),
              field(
                'exe_path',
                'Executable or application bundle',
                browse: true,
              ),
              DropdownButtonFormField<String>(
                initialValue: value['kind'],
                items: const [
                  DropdownMenuItem(
                    value: 'windows',
                    child: Text('Windows game'),
                  ),
                  DropdownMenuItem(value: 'linux', child: Text('Linux native')),
                  DropdownMenuItem(
                    value: 'native',
                    child: Text('Native application'),
                  ),
                  DropdownMenuItem(value: 'macos', child: Text('macOS native')),
                ],
                onChanged: (v) => setState(() => value['kind'] = v),
                decoration: const InputDecoration(labelText: 'Type'),
              ),
              const SizedBox(height: 12),
              field('category', 'Category'),
              field('arguments', 'Arguments'),
              field(
                'working_directory',
                'Working directory',
                browse: true,
                directory: true,
              ),
              field('additional_app', 'Additional application', browse: true),
              field('cover_path', 'Cover image', browse: true),
              if (widget.state.compatibility && value['kind'] == 'windows') ...[
                DropdownButtonFormField<String>(
                  initialValue:
                      (widget.state.catalogue['runners'] as List? ?? []).any(
                        (r) => r['id'] == value['runner'],
                      )
                      ? value['runner']
                      : null,
                  items: (widget.state.catalogue['runners'] as List? ?? [])
                      .map(
                        (r) => DropdownMenuItem<String>(
                          value: r['id'],
                          child: Text(
                            '${r['name']}${r['available'] == true ? '' : ' (not installed)'}',
                          ),
                        ),
                      )
                      .toList(),
                  onChanged: (v) => value['runner'] = v,
                  decoration: const InputDecoration(
                    labelText: 'Compatibility runner',
                  ),
                ),
                const SizedBox(height: 12),
                field(
                  'prefix_path',
                  'Wine prefix (empty = isolated default)',
                  browse: true,
                  directory: true,
                ),
              ],
              if (widget.state.compatibility) ...[
                ExpansionTile(
                  title: const Text('Compatibility and performance'),
                  children: launchToggles.entries
                      .where(
                        (entry) =>
                            value['kind'] == 'windows' ||
                            const [
                              'mangohud',
                              'gamemode',
                              'gamescope',
                              'prefer_sdl',
                            ].contains(entry.key),
                      )
                      .map(
                        (entry) => SwitchListTile(
                          title: Text(entry.value),
                          value: value[entry.key] == true,
                          onChanged: (v) =>
                              setState(() => value[entry.key] = v),
                        ),
                      )
                      .toList(),
                ),
                if (value['virtual_desktop'] == true)
                  field(
                    'virtual_desktop_size',
                    'Virtual desktop size, e.g. 1920x1080',
                  ),
              ],
              field(
                'environment',
                'Environment (NAME=value, one per line)',
                lines: 3,
              ),
            ],
          ),
        ),
      ),
    ),
    actions: [
      TextButton(
        onPressed: saving ? null : () => Navigator.pop(context),
        child: const Text('Cancel'),
      ),
      FilledButton(
        onPressed: saving ? null : save,
        child: Text(saving ? 'Saving…' : 'Save'),
      ),
    ],
  );
}
