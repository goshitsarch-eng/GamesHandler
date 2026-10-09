import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:url_launcher/url_launcher.dart';

import '../state/library_state.dart';

typedef RunAction = Future<void> Function(Future<void> Function());
List<Map<String, dynamic>> rows(LibraryState s, String key) =>
    (s.catalogue[key] as List? ?? [])
        .map((v) => Map<String, dynamic>.from(v))
        .toList();
Future<void> openLink(String value) async {
  if (!await launchUrl(
    Uri.parse(value),
    mode: LaunchMode.externalApplication,
  )) {
    throw StateError('Could not open $value');
  }
}

class InstallersPage extends StatefulWidget {
  const InstallersPage({
    super.key,
    required this.state,
    required this.onAction,
  });
  final LibraryState state;
  final RunAction onAction;
  @override
  State<InstallersPage> createState() => _InstallersPageState();
}

class _InstallersPageState extends State<InstallersPage> {
  String search = '', category = 'All';
  String? runner;
  @override
  Widget build(BuildContext context) {
    final installers = rows(widget.state, 'installers');
    final categories = {
      'All',
      ...installers.map((i) => i['category'] as String? ?? 'Apps'),
    };
    final runners = rows(widget.state, 'runners');
    return ListView(
      padding: const EdgeInsets.all(20),
      children: [
        Text(
          'Install store launchers',
          style: Theme.of(context).textTheme.headlineSmall,
        ),
        const SizedBox(height: 12),
        if (!widget.state.compatibility)
          Text(
            widget.state.catalogue['compatibility_note'] ??
                'Use the official native installer.',
          ),
        TextField(
          decoration: const InputDecoration(
            labelText: 'Search installers',
            prefixIcon: Icon(Icons.search),
          ),
          onChanged: (v) => setState(() => search = v),
        ),
        Wrap(
          spacing: 16,
          children: [
            DropdownButton<String>(
              value: category,
              items: categories
                  .map((c) => DropdownMenuItem(value: c, child: Text(c)))
                  .toList(),
              onChanged: (v) => setState(() => category = v ?? 'All'),
            ),
            if (widget.state.compatibility)
              DropdownButton<String>(
                hint: const Text('Choose runner'),
                value: runners.any((r) => r['id'] == runner) ? runner : null,
                items: runners
                    .map(
                      (r) => DropdownMenuItem<String>(
                        value: r['id'],
                        child: Text(r['name']),
                      ),
                    )
                    .toList(),
                onChanged: (v) => setState(() => runner = v),
              ),
          ],
        ),
        ...installers
            .where(
              (i) =>
                  (category == 'All' ||
                      (i['category'] ?? 'Apps') == category) &&
                  '${i['name']} ${i['description']}'.toLowerCase().contains(
                    search.toLowerCase(),
                  ),
            )
            .map(
              (i) => Card(
                child: ListTile(
                  title: Text(i['name']),
                  subtitle: Text('${i['description']}\n${i['notes'] ?? ''}'),
                  trailing: FilledButton.tonal(
                    onPressed: () => widget.onAction(() async {
                      if (widget.state.compatibility) {
                        await widget.state.startJob('installer', {
                          'id': i['id'],
                          'runner':
                              runner ??
                              widget.state.settings['default_runner'] ??
                              'wine-system',
                        });
                      } else {
                        await openLink(i['url']);
                      }
                    }),
                    child: Text(
                      widget.state.compatibility
                          ? 'Install'
                          : 'Official installer',
                    ),
                  ),
                ),
              ),
            ),
      ],
    );
  }
}

class RunnersPage extends StatefulWidget {
  const RunnersPage({super.key, required this.state, required this.onAction});
  final LibraryState state;
  final RunAction onAction;
  @override
  State<RunnersPage> createState() => _RunnersPageState();
}

class _RunnersPageState extends State<RunnersPage> {
  String family = 'proton-ge';
  List<Map<String, dynamic>> releases = [];
  bool loading = false;
  String? error;
  int token = 0;
  @override
  void initState() {
    super.initState();
    if (widget.state.compatibility) fetch();
  }

  Future<void> fetch() async {
    final issued = ++token;
    setState(() {
      loading = true;
      error = null;
    });
    try {
      final value = await widget.state.backend.call('releases', {
        'family': family,
      }) as List;
      if (mounted && issued == token) {
        setState(
          () => releases = value
              .map((v) => Map<String, dynamic>.from(v))
              .toList(),
        );
      }
    } catch (e) {
      if (mounted && issued == token) setState(() => error = e.toString());
    } finally {
      if (mounted && issued == token) setState(() => loading = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    if (!widget.state.compatibility) {
      return const Center(
        child: Padding(
          padding: EdgeInsets.all(24),
          child: Text(
            'Wine and Proton distributions managed here are Linux compatibility tools. Native Windows and macOS games launch directly from your library.',
          ),
        ),
      );
    }
    final families = rows(widget.state, 'families');
    return ListView(
      padding: const EdgeInsets.all(20),
      children: [
        Text(
          'Installed runners',
          style: Theme.of(context).textTheme.headlineSmall,
        ),
        ...rows(widget.state, 'runners').map(
          (r) => ListTile(
            title: Text(r['name']),
            subtitle: Text(
              r['available'] == true ? 'Available' : 'Not installed',
            ),
            trailing: r['id'] == 'wine-system'
                ? null
                : IconButton(
                    tooltip: 'Remove ${r['name']}',
                    icon: const Icon(Icons.delete_outline),
                    onPressed: () => widget.onAction(() async {
                      final yes = await showDialog<bool>(
                        context: context,
                        builder: (c) => AlertDialog(
                          title: Text('Remove ${r['name']}?'),
                          content: const Text(
                            'The downloaded runner will be deleted. Game files and prefixes are kept.',
                          ),
                          actions: [
                            TextButton(
                              onPressed: () => Navigator.pop(c, false),
                              child: const Text('Cancel'),
                            ),
                            FilledButton(
                              onPressed: () => Navigator.pop(c, true),
                              child: const Text('Remove'),
                            ),
                          ],
                        ),
                      );
                      if (yes == true) {
                        await widget.state.backend.call('remove_runner', {
                          'id': r['id'],
                        });
                        await widget.state.reloadCatalogue();
                      }
                    }),
                  ),
          ),
        ),
        const Divider(),
        Text(
          'Download a build',
          style: Theme.of(context).textTheme.headlineSmall,
        ),
        Wrap(
          spacing: 16,
          crossAxisAlignment: WrapCrossAlignment.center,
          children: [
            DropdownButton<String>(
              value: families.any((f) => f['id'] == family) ? family : null,
              items: families
                  .map(
                    (f) => DropdownMenuItem<String>(
                      value: f['id'],
                      child: Text(f['name']),
                    ),
                  )
                  .toList(),
              onChanged: (v) {
                if (v != null) {
                  setState(() => family = v);
                  fetch();
                }
              },
            ),
            TextButton.icon(
              onPressed: loading ? null : fetch,
              icon: const Icon(Icons.refresh),
              label: const Text('Refresh'),
            ),
          ],
        ),
        ...families
            .where((f) => f['id'] == family)
            .map(
              (f) => ListTile(
                title: Text(f['description']),
                subtitle: Text(f['guide']),
                trailing: TextButton(
                  onPressed: () => widget.onAction(() => openLink(f['url'])),
                  child: const Text('Visit project'),
                ),
              ),
            ),
        if (loading) const LinearProgressIndicator(),
        if (error != null)
          Text(
            error!,
            style: TextStyle(color: Theme.of(context).colorScheme.error),
          ),
        if (!loading && error == null && releases.isEmpty)
          const Text(
            'No compatible releases were found for this architecture.',
          ),
        ...releases.map(
          (r) => ListTile(
            title: Text(r['name']),
            subtitle: Text(
              '${r['tag']} · ${((r['size'] as num) / 1048576).toStringAsFixed(0)} MB',
            ),
            trailing: FilledButton.tonal(
              onPressed: () => widget.onAction(
                () => widget.state.startJob('runner', {
                  'family': family,
                  'tag': r['tag'],
                }),
              ),
              child: const Text('Install'),
            ),
          ),
        ),
      ],
    );
  }
}

class PluginsPage extends StatelessWidget {
  const PluginsPage({super.key, required this.state, required this.onAction});
  final LibraryState state;
  final RunAction onAction;
  @override
  Widget build(BuildContext context) => ListView(
    padding: const EdgeInsets.all(20),
    children: [
      Text(
        'Optional compatibility tools',
        style: Theme.of(context).textTheme.headlineSmall,
      ),
      if (!state.compatibility)
        const Text(
          'MangoHud, GameMode, Gamescope, Winetricks and UMU integrate with the Linux compatibility workflow. They are not required for native games on this platform.',
        ),
      ...rows(state, 'plugins').map(
        (p) => Card(
          child: ListTile(
            title: Text('${p['name']} · ${p['status']}'),
            subtitle: Text(p['description']),
            trailing: p['status'] == 'installed'
                ? const Icon(Icons.check_circle_outline)
                : TextButton(
                    onPressed: () => onAction(() async {
                      final command = await state.backend.call(
                        'plugin_command',
                        {'id': p['id']},
                      );
                      if (!context.mounted) return;
                      await showDialog<void>(
                        context: context,
                        builder: (c) => AlertDialog(
                          title: Text('Install ${p['name']}'),
                          content: SelectableText(
                            'Run this command in your system terminal:\n\n${command['display']}',
                          ),
                          actions: [
                            TextButton(
                              onPressed: () => Clipboard.setData(
                                ClipboardData(text: command['display']),
                              ),
                              child: const Text('Copy command'),
                            ),
                            FilledButton(
                              onPressed: () => Navigator.pop(c),
                              child: const Text('Close'),
                            ),
                          ],
                        ),
                      );
                    }),
                    child: const Text('Install instructions'),
                  ),
          ),
        ),
      ),
      TextButton.icon(
        onPressed: () => onAction(state.reloadCatalogue),
        icon: const Icon(Icons.refresh),
        label: const Text('Refresh detection'),
      ),
    ],
  );
}

class AboutPage extends StatelessWidget {
  const AboutPage({super.key, required this.state, required this.onAction});
  final LibraryState state;
  final RunAction onAction;
  @override
  Widget build(BuildContext context) => ListView(
    padding: const EdgeInsets.all(20),
    children: [
      Text(
        'GameHandler ${state.version}',
        style: Theme.of(context).textTheme.headlineMedium,
      ),
      const Text(
        'Made by Gosh.\nA game library and compatibility front-end. GameHandler does not implement Windows compatibility or guarantee game compatibility.',
      ),
      const SizedBox(height: 16),
      const Text(
        'Flutter desktop migration — prerelease. Flutter/Dart provide the desktop UI; Rust provides domain operations. Legacy toolkit credits below acknowledge the retained reference implementation.',
      ),
      ...const [
        (
          'Flutter',
          'Flutter contributors',
          'BSD-3-Clause',
          'https://flutter.dev',
        ),
        ('Dart', 'Dart contributors', 'BSD-3-Clause', 'https://dart.dev'),
        (
          'Rust',
          'Rust contributors',
          'MIT OR Apache-2.0',
          'https://www.rust-lang.org',
        ),
      ].map(
        (entry) => ListTile(
          title: Text('${entry.$1} · ${entry.$2}'),
          subtitle: Text(entry.$3),
          trailing: TextButton(
            onPressed: () => onAction(() => openLink(entry.$4)),
            child: const Text('Visit'),
          ),
        ),
      ),
      TextButton(
        onPressed: () => showLicensePage(
          context: context,
          applicationName: 'GameHandler',
          applicationVersion: state.version,
        ),
        child: const Text('Open-source licenses'),
      ),
      ...rows(state, 'credits').map(
        (s) => Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            const Divider(),
            Text(s['title'], style: Theme.of(context).textTheme.headlineSmall),
            Text(s['summary']),
            ...(s['entries'] as List).map(
              (e) => ListTile(
                title: Text('${e['name']} · ${e['authors']}'),
                subtitle: Text('${e['description']}\n${e['license']}'),
                trailing: TextButton(
                  onPressed: () => onAction(() => openLink(e['url'])),
                  child: const Text('Visit'),
                ),
              ),
            ),
          ],
        ),
      ),
    ],
  );
}
