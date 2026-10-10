import 'dart:io';

import 'package:flutter/material.dart';

import '../models/game.dart';
import '../state/library_state.dart';

class LibraryPage extends StatelessWidget {
  const LibraryPage({
    super.key,
    required this.state,
    required this.searchFocus,
    required this.onAdd,
    required this.onAction,
  });
  final LibraryState state;
  final FocusNode searchFocus;
  final VoidCallback onAdd;
  final void Function(GameDto, String) onAction;
  @override
  Widget build(BuildContext context) => CustomScrollView(
    slivers: [
      SliverToBoxAdapter(child: _toolbar(context)),
      if (state.loading && state.games.isEmpty)
        const SliverFillRemaining(
          hasScrollBody: false,
          child: Center(
            child: CircularProgressIndicator(semanticsLabel: 'Loading library'),
          ),
        )
      else if (state.games.isEmpty)
        SliverFillRemaining(
          hasScrollBody: false,
          child: Center(
            child: Padding(
              padding: const EdgeInsets.all(16),
              child: Column(
                mainAxisSize: MainAxisSize.min,
                children: [
                  Text(
                    state.query.isEmpty
                        ? 'No games yet'
                        : 'No games match your filters',
                    style: Theme.of(context).textTheme.headlineSmall,
                    textAlign: TextAlign.center,
                  ),
                  const SizedBox(height: 12),
                  const Text(
                    'Add an executable or drop it onto this window.',
                    textAlign: TextAlign.center,
                  ),
                  const SizedBox(height: 16),
                  FilledButton(onPressed: onAdd, child: const Text('Add game')),
                ],
              ),
            ),
          ),
        )
      else if (state.settings['view_mode'] == 'list')
        SliverList.builder(
          itemCount: state.games.length,
          itemBuilder: (c, i) => _row(c, state.games[i]),
        )
      else
        SliverPadding(
          padding: const EdgeInsets.all(16),
          sliver: SliverGrid.builder(
            gridDelegate: SliverGridDelegateWithMaxCrossAxisExtent(
              maxCrossAxisExtent: 260,
              mainAxisExtent:
                  280 * MediaQuery.textScalerOf(context).scale(1).clamp(1, 3),
              mainAxisSpacing: 16,
              crossAxisSpacing: 16,
            ),
            itemCount: state.games.length,
            itemBuilder: (c, i) => _card(c, state.games[i]),
          ),
        ),
    ],
  );
  Widget _toolbar(BuildContext context) => Padding(
    padding: const EdgeInsets.all(16),
    child: Wrap(
      spacing: 12,
      runSpacing: 12,
      crossAxisAlignment: WrapCrossAlignment.center,
      children: [
        SizedBox(
          width: 300,
          child: TextField(
            focusNode: searchFocus,
            onChanged: state.search,
            decoration: const InputDecoration(
              labelText: 'Search games',
              prefixIcon: Icon(Icons.search),
              border: OutlineInputBorder(),
            ),
          ),
        ),
        SizedBox(
          width: 160,
          child: DropdownButton<String>(
            isExpanded: true,
            value: ['All', ...state.categories].contains(state.category)
                ? state.category
                : 'All',
            items: ['All', ...state.categories.where((c) => c != 'All')]
                .map(
                  (c) => DropdownMenuItem(
                    value: c,
                    child: Text(
                      c,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                    ),
                  ),
                )
                .toList(),
            onChanged: (c) {
              state.category = c ?? 'All';
              state.refresh();
            },
          ),
        ),
        SizedBox(
          width: 200,
          child: DropdownButton<String>(
            isExpanded: true,
            value: state.sort,
            items: const [
              DropdownMenuItem(
                value: 'name',
                child: Text(
                  'Name',
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                ),
              ),
              DropdownMenuItem(
                value: 'recent',
                child: Text(
                  'Recently played',
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                ),
              ),
              DropdownMenuItem(
                value: 'added',
                child: Text(
                  'Recently added',
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                ),
              ),
            ],
            onChanged: (v) {
              state.sort = v ?? 'name';
              state.refresh();
            },
          ),
        ),
        IconButton(
          tooltip: state.settings['view_mode'] == 'list'
              ? 'Grid view'
              : 'List view',
          onPressed: () => state.saveSettings(
            'view_mode',
            state.settings['view_mode'] == 'list' ? 'grid' : 'list',
          ),
          icon: Icon(
            state.settings['view_mode'] == 'list'
                ? Icons.grid_view
                : Icons.view_list,
          ),
        ),
        FilledButton.icon(
          onPressed: onAdd,
          icon: const Icon(Icons.add),
          label: const Text('Add game'),
        ),
      ],
    ),
  );
  Widget _cover(BuildContext c, GameDto g) => g.cover.isEmpty
      ? Center(
          child: Text(
            g.name.isEmpty ? '?' : g.name.characters.first.toUpperCase(),
            style: Theme.of(c).textTheme.displayLarge,
          ),
        )
      : Image.file(
          File(g.cover),
          fit: BoxFit.cover,
          cacheWidth: 480,
          key: ValueKey('${g.id}:${g.cover}:${state.artworkRevision}'),
          errorBuilder: (_, error, stack) => const Center(
            child: Icon(Icons.image_not_supported_outlined, size: 48),
          ),
        );
  Widget _menu(GameDto g) => PopupMenuButton<String>(
    tooltip: 'Actions for ${g.name}',
    onSelected: (v) => onAction(g, v),
    itemBuilder: (c) => _items(g),
  );
  List<PopupMenuEntry<String>> _items(GameDto g) => [
    const PopupMenuItem(value: 'play', child: Text('Play')),
    const PopupMenuItem(value: 'edit', child: Text('Edit')),
    const PopupMenuItem(value: 'fetch_cover', child: Text('Find cover art')),
    const PopupMenuItem(
      value: 'custom_cover',
      child: Text('Choose cover image'),
    ),
    if (state.compatibility && g.kind == 'windows') ...const [
      PopupMenuItem(value: 'winecfg', child: Text('Wine configuration')),
      PopupMenuItem(value: 'winetricks', child: Text('Winetricks')),
      PopupMenuItem(value: 'prefix', child: Text('Open prefix folder')),
    ],
    const PopupMenuItem(
      value: 'shortcut',
      child: Text('Create desktop shortcut'),
    ),
    const PopupMenuDivider(),
    const PopupMenuItem(value: 'remove', child: Text('Remove from library')),
  ];
  Widget _context(BuildContext c, GameDto g, Widget child) => GestureDetector(
    onSecondaryTapDown: (details) async {
      final overlay = Overlay.of(c).context.findRenderObject() as RenderBox;
      final value = await showMenu<String>(
        context: c,
        position: RelativeRect.fromRect(
          Rect.fromLTWH(
            details.globalPosition.dx,
            details.globalPosition.dy,
            1,
            1,
          ),
          Offset.zero & overlay.size,
        ),
        items: _items(g),
      );
      if (value != null) onAction(g, value);
    },
    child: child,
  );
  Widget _row(BuildContext c, GameDto g) => _context(
    c,
    g,
    ListTile(
      leading: SizedBox(width: 48, height: 48, child: _cover(c, g)),
      title: Text(g.name),
      subtitle: Text('${g.category} · ${g.kind}'),
      onTap: () => onAction(g, 'play'),
      trailing: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          IconButton(
            tooltip: 'Play ${g.name}',
            onPressed: () => onAction(g, 'play'),
            icon: const Icon(Icons.play_arrow),
          ),
          _menu(g),
        ],
      ),
    ),
  );
  Widget _card(BuildContext c, GameDto g) => _context(
    c,
    g,
    Card(
      clipBehavior: Clip.antiAlias,
      child: Column(
        children: [
          Expanded(
            child: InkWell(
              onDoubleTap: () => onAction(g, 'play'),
              child: SizedBox(width: double.infinity, child: _cover(c, g)),
            ),
          ),
          ListTile(
            title: Text(g.name, maxLines: 1, overflow: TextOverflow.ellipsis),
            subtitle: Text(g.category, maxLines: 1),
            trailing: _menu(g),
          ),
          Padding(
            padding: const EdgeInsets.only(bottom: 8),
            child: FilledButton.tonalIcon(
              onPressed: () => onAction(g, 'play'),
              icon: const Icon(Icons.play_arrow),
              label: const Text('Play'),
            ),
          ),
        ],
      ),
    ),
  );
}
