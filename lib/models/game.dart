/// A read-only bridge DTO. Rust owns validation and persistence.
class GameDto {
  GameDto(Map<String, dynamic> value) : fields = Map.unmodifiable(value);
  final Map<String, dynamic> fields;
  String get id => fields['id'] as String? ?? '';
  String get name => fields['name'] as String? ?? '';
  String get executable => fields['exe_path'] as String? ?? '';
  String get cover => fields['cover_path'] as String? ?? '';
  String get category => fields['category'] as String? ?? 'Uncategorized';
  String get kind => fields['kind'] as String? ?? 'windows';
}

const launchToggles = {
  'mangohud': 'MangoHud',
  'gamemode': 'GameMode',
  'prefer_sdl': 'Prefer SDL',
  'wayland': 'Wine Wayland',
  'hdr': 'HDR',
  'esync': 'Esync',
  'fsync': 'Fsync',
  'dxvk': 'DXVK',
  'vkd3d': 'VKD3D',
  'nvapi': 'NVAPI / DLSS',
  'fsr': 'AMD FSR',
  'battleye': 'BattlEye runtime',
  'eac': 'Easy Anti-Cheat runtime',
  'gamescope': 'Gamescope',
  'virtual_desktop': 'Virtual desktop',
};
