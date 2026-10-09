import 'package:flutter/material.dart';

import '../models/game.dart';
import '../state/library_state.dart';
import 'catalogue.dart';

class SettingsPage extends StatelessWidget {
  const SettingsPage({super.key, required this.state, required this.onAction});
  final LibraryState state;
  final RunAction onAction;
  Widget select(String key, String title, Map<String, String> choices) =>
      DropdownButtonFormField<String>(
        key: ValueKey('$key:${state.settings[key]}'),
        initialValue: choices.containsKey(state.settings[key])
            ? state.settings[key]
            : choices.keys.first,
        decoration: InputDecoration(labelText: title),
        items: choices.entries
            .map((e) => DropdownMenuItem(value: e.key, child: Text(e.value)))
            .toList(),
        onChanged: (v) {
          if (v != null) onAction(() => state.saveSettings(key, v));
        },
      );
  @override
  Widget build(BuildContext context) => ListView(
    padding: const EdgeInsets.all(20),
    children: [
      Text('Appearance', style: Theme.of(context).textTheme.headlineSmall),
      const SizedBox(height: 12),
      select('color_scheme', 'Theme', {
        'system': 'Follow system',
        'light': 'Light',
        'dark': 'Dark',
      }),
      const SizedBox(height: 12),
      select('view_mode', 'Library layout', {'grid': 'Grid', 'list': 'List'}),
      const SizedBox(height: 12),
      select('sort_mode', 'Default sort', {
        'name': 'Name',
        'recent': 'Recently played',
        'added': 'Recently added',
      }),
      const SizedBox(height: 24),
      SwitchListTile(
        title: const Text('Hide window after launching a game'),
        subtitle: const Text('The game keeps running independently.'),
        value: state.settings['close_on_launch'] == true,
        onChanged: (v) =>
            onAction(() => state.saveSettings('close_on_launch', v)),
      ),
      if (state.compatibility) ...[
        const Divider(),
        Text('New games', style: Theme.of(context).textTheme.headlineSmall),
        select('default_runner', 'Default runner', {
          for (final r in rows(state, 'runners')) r['id']: r['name'],
        }),
        ...launchToggles.entries
            .where((e) => state.settings.containsKey('default_${e.key}'))
            .map(
              (entry) => SwitchListTile(
                title: Text('Enable ${entry.value} by default'),
                value: state.settings['default_${entry.key}'] == true,
                onChanged: (v) => onAction(
                  () => state.saveSettings('default_${entry.key}', v),
                ),
              ),
            ),
      ],
      const Divider(),
      Text(
        'Keyboard shortcuts',
        style: Theme.of(context).textTheme.headlineSmall,
      ),
      Text(
        '${state.platform == 'macos' ? 'Command' : 'Ctrl'}+N: Add game\n${state.platform == 'macos' ? 'Command' : 'Ctrl'}+F: Search\n${state.platform == 'macos' ? 'Command' : 'Ctrl'}+,: Settings\n${state.platform == 'macos' ? 'Command' : 'Ctrl'}+Q: Quit',
      ),
    ],
  );
}
