const indexUrl = 'maps/index.json';
const grid = document.getElementById('map-grid');
const status = document.getElementById('status');
const search = document.getElementById('search');
let maps = [];

function text(value, fallback = '') {
  return typeof value === 'string' && value.trim() ? value : fallback;
}

function duration(value) {
  const seconds = Number(value);
  if (!Number.isFinite(seconds) || seconds <= 0) return '—';
  return `${Math.floor(seconds / 60)}:${String(Math.floor(seconds % 60)).padStart(2, '0')}`;
}

function sourceHost(url) {
  try { return new URL(url).hostname.replace(/^www\./, ''); } catch { return 'source link'; }
}

function fallbackLetter(title) {
  return text(title, '?').trim().charAt(0).toUpperCase() || '?';
}

function render() {
  const query = search.value.trim().toLowerCase();
  const visible = maps.filter((map) => {
    if (!query) return true;
    return [map.title, map.artist, map.album].some((value) =>
      text(value).toLowerCase().includes(query)
    );
  });
  grid.replaceChildren();
  status.textContent = visible.length === 1
    ? '1 map'
    : `${visible.length} maps`;

  if (!visible.length) {
    const empty = document.createElement('div');
    empty.className = 'empty';
    empty.textContent = maps.length
      ? 'No maps match that search.'
      : 'No community maps yet.';
    grid.append(empty);
    return;
  }

  for (const map of visible) {
    const card = document.createElement('article');
    card.className = 'card';

    const cover = document.createElement('div');
    cover.className = 'cover';
    if (text(map.cover)) {
      const img = document.createElement('img');
      img.src = map.cover.startsWith('data:') ? map.cover : `maps/${map.cover}`;
      img.alt = `Cover art for ${text(map.title, 'map')}`;
      img.loading = 'lazy';
      img.onerror = () => { img.remove(); cover.append(fallback(map)); };
      cover.append(img);
    } else {
      cover.append(fallback(map));
    }

    const body = document.createElement('div');
    body.className = 'card-body';
    const title = document.createElement('h3');
    title.textContent = text(map.title, 'Untitled map');
    const byline = document.createElement('div');
    byline.className = 'byline';
    byline.textContent = [text(map.artist, 'Unknown artist'), text(map.album)].filter(Boolean).join(' · ');
    const meta = document.createElement('div');
    meta.className = 'meta';
    meta.textContent = `${duration(map.duration)} · ${Number(map.notes) || 0} notes · ${Number(map.lyrics) || 0} lyric lines`;
    const source = document.createElement('div');
    source.className = 'meta';
    source.textContent = `source: ${sourceHost(map.sourceUrl)}`;
    const actions = document.createElement('div');
    actions.className = 'actions';
    const link = document.createElement('a');
    link.href = `maps/${encodeURIComponent(map.file)}`;
    link.download = map.file;
    link.textContent = 'DOWNLOAD MAP';
    actions.append(link);
    body.append(title, byline, meta, source, actions);
    card.append(cover, body);
    grid.append(card);
  }
}

function fallback(map) {
  const el = document.createElement('div');
  el.className = 'fallback';
  el.textContent = fallbackLetter(map.title);
  return el;
}

async function load() {
  try {
    const response = await fetch(indexUrl, { cache: 'no-store' });
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    const data = await response.json();
    maps = (Array.isArray(data) ? data : data.maps || [])
      .filter((map) => map && map.file) 
      .sort((a, b) => text(a.title).localeCompare(text(b.title)));
    render();
  } catch (error) {
    status.textContent = 'Could not load the map index.';
    grid.innerHTML = `<div class="empty">The map index is unavailable. Open this page from GitHub Pages or serve the site directory locally.<br><br>${String(error.message || error)}</div>`;
  }
}

search.addEventListener('input', render);
load();
