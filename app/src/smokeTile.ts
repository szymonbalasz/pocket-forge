// `tauri:smoke` tile step (BL-07, R46): runs only when a debug build with PF_SMOKE hands the UI
// an image through `app_ready`. Opens it through the core as a map project and loads tile 0/0_0
// through the `pf-tiles` scheme the way the editor does (fetch + createImageBitmap), so the
// scheme, its registration and the CSP are checked end to end.
import { api } from './api';

export async function smokeTile(image: string): Promise<{ ok: boolean; detail: string }> {
  try {
    const { items } = await api.project.create('maps', [image]);
    const item = items[0];
    if (!item || item.status === 'skipped')
      throw new Error(`project_create: ${JSON.stringify(item)}`);
    const project = await api.project.open(item.path);
    const original = project.layers.find((l) => l.kind === 'original') ?? project.layers[0];
    if (!original) throw new Error('the project has no original layer');
    const desc = await api.editor.tilesOpen(project.project_id, original.id);
    const lv = desc.levels[0];
    if (!lv) throw new Error('the pyramid has no levels');
    const url = `${desc.base_url}/${lv.level}/0_0.${lv.ext}`;
    // Level 0 may still be building: a missing tile is a 404 until it lands.
    for (let i = 0; ; i++) {
      const res = await fetch(url);
      if (res.ok) {
        const type = res.headers.get('content-type') ?? '';
        const bitmap = await createImageBitmap(await res.blob());
        const want = Math.min(desc.tile_size, desc.width);
        const ok = type.startsWith('image/') && bitmap.width === want;
        const detail = `tile ${url} ${res.status} ${type} ${bitmap.width}x${bitmap.height}`;
        bitmap.close();
        return { ok, detail };
      }
      if (i >= 150) return { ok: false, detail: `tile ${url} still ${res.status} after 30 s` };
      await new Promise((r) => setTimeout(r, 200));
    }
  } catch (e) {
    return { ok: false, detail: `tile step: ${(e as Error).message ?? String(e)}` };
  }
}
