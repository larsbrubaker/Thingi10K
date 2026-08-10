'use strict';

// ── State ───────────────────────────────────────────────────────────────────
let scene, camera, renderer, controls, currentMesh, currentBed;
let selectedId        = null;
let isWireframe       = false;
let currentDetailModel = null;
let allModels         = [];  // populated from data/models.json at startup

let sortKey = 'id';
let sortDir = 'asc';

// Tri-state filter values: 'both' | 'true' | 'false'
const filterState = {
  closed:           'both',
  edge_manifold:    'both',
  vertex_manifold:  'both',
  single_component: 'both',
  pwn:              'both',
};

// Polygon (face) range slider state — set from data at load time
let polyMin    = 0;
let polyMax    = 0;
let polyAbsMin = 0;  // current slider bounds (change with filter set)
let polyAbsMax = 1;

// ── Mesh CDN ────────────────────────────────────────────────────────────────
// `?cdn=<base>` overrides the CDN for local testing.
const MESH_CDN = new URLSearchParams(location.search).get('cdn')
  || 'https://cdn.jsdelivr.net/gh/larsbrubaker';

// ── Deep link ───────────────────────────────────────────────────────────────
// `?model=<id>` opens straight to one model, so links are shareable from
// other tools. Parsed here; applied after the catalog loads (see Boot).
const DEEP_LINK_ID = (() => {
  const raw = new URLSearchParams(location.search).get('model');
  if (raw === null || !/^\d+$/.test(raw.trim())) return null;   // unknown/garbage → ignore
  return parseInt(raw, 10);
})();

// Keep the address bar in sync with the current selection without adding
// history entries (back should leave the page, not walk the click history).
// Every other param — notably `cdn` — is preserved.
function syncUrlToSelection(id) {
  const params = new URLSearchParams(location.search);
  params.set('model', String(id));
  history.replaceState(null, '', `${location.pathname}?${params}${location.hash}`);
}

function scrollListEntryIntoView(id) {
  const el = document.querySelector(`.model-item[data-id="${id}"]`);
  if (el) el.scrollIntoView({ block: 'center' });
}

// Meshes whose zip exceeds jsDelivr's ~20 MB limit are split into parts:
// ID.ext_1.zip … ID.ext_N.zip, each holding a 15 MiB slice of the raw file.
// `model.parts` (from models.json) is N; absent means a single ID.ext.zip.
function meshZipUrl(model, part) {
  const suffix = part ? `_${part}` : '';
  return `${MESH_CDN}/Thingi10K-meshes-${model.repo}@main/meshes/${model.id}.${model.format}${suffix}.zip`;
}

async function fetchModelBuffer(model) {
  if (!model.parts) return fetchAndDecompress(meshZipUrl(model));
  const parts = await Promise.all(
    Array.from({ length: model.parts }, (_, i) => fetchAndDecompress(meshZipUrl(model, i + 1)))
  );
  const total  = parts.reduce((n, b) => n + b.byteLength, 0);
  const joined = new Uint8Array(total);
  let offset = 0;
  for (const b of parts) {
    joined.set(new Uint8Array(b), offset);
    offset += b.byteLength;
  }
  return joined.buffer;
}

// ── fflate decompression ────────────────────────────────────────────────────
async function fetchAndDecompress(url) {
  const resp = await fetch(url);
  if (!resp.ok) throw new Error(`HTTP ${resp.status} — ${url}`);
  const compressed = new Uint8Array(await resp.arrayBuffer());
  return new Promise((resolve, reject) => {
    fflate.unzip(compressed, (err, files) => {
      if (err) return reject(err);
      const data = Object.values(files)[0];
      resolve(data.buffer.slice(data.byteOffset, data.byteOffset + data.byteLength));
    });
  });
}

// ── Binary STL repair ───────────────────────────────────────────────────────
// Some files in the dataset have a face-count header that is 1 larger than
// the actual data (the file is truncated by exactly 50 bytes).  STLLoader
// throws a DataView RangeError when it tries to read that missing face.
// Fix by clamping the header to the number of complete 50-byte records.
function fixTruncatedBinaryStl(buffer) {
  if (buffer.byteLength < 84) return buffer;
  const view    = new DataView(buffer);
  const nFaces  = view.getUint32(80, true);
  const expected = 80 + 4 + nFaces * 50;
  if (expected <= buffer.byteLength) return buffer;       // file is fine
  const actualFaces = Math.floor((buffer.byteLength - 84) / 50);
  const fixed = buffer.slice(0);                          // copy so we can mutate
  new DataView(fixed).setUint32(80, actualFaces, true);
  return fixed;
}

// ── Three.js init ───────────────────────────────────────────────────────────
function initThree() {
  const container = document.getElementById('viewer');

  scene = new THREE.Scene();
  scene.background = new THREE.Color(0xeceae5);

  camera = new THREE.PerspectiveCamera(
    45,
    container.clientWidth / container.clientHeight,
    0.01,
    100000
  );
  camera.up.set(0, 0, 1);
  camera.position.set(0, -100, 50);

  renderer = new THREE.WebGLRenderer({ antialias: true });
  renderer.setPixelRatio(window.devicePixelRatio);
  renderer.setSize(container.clientWidth, container.clientHeight);
  container.appendChild(renderer.domElement);

  scene.add(new THREE.AmbientLight(0xffffff, 0.6));
  const key = new THREE.DirectionalLight(0xffffff, 0.85);
  key.position.set(2, -3, 4);
  scene.add(key);
  const fill = new THREE.DirectionalLight(0xfff8f0, 0.3);
  fill.position.set(-2, 1, -3);
  scene.add(fill);

  controls = new THREE.OrbitControls(camera, renderer.domElement);
  controls.enableDamping = true;
  controls.dampingFactor = 0.06;

  new ResizeObserver(() => {
    camera.aspect = container.clientWidth / container.clientHeight;
    camera.updateProjectionMatrix();
    renderer.setSize(container.clientWidth, container.clientHeight);
  }).observe(container);

  (function animate() {
    requestAnimationFrame(animate);
    controls.update();
    renderer.render(scene, camera);
  })();
}

// ── Camera fit ──────────────────────────────────────────────────────────────
function fitCamera(box) {
  const center = box.getCenter(new THREE.Vector3());
  const size   = box.getSize(new THREE.Vector3());
  const maxDim = Math.max(size.x, size.y, size.z);
  const fov    = camera.fov * (Math.PI / 180);
  const dist   = Math.abs(maxDim / (2 * Math.tan(fov / 2))) * 1.6;

  camera.position.copy(center).add(
    new THREE.Vector3(dist * 0.6, -dist, dist * 0.7)
  );
  camera.near = dist / 100;
  camera.far  = dist * 100;
  camera.updateProjectionMatrix();
  controls.target.copy(center);
  controls.update();
  addBed(box);
}

// ── Build plate ─────────────────────────────────────────────────────────────
function addBed(box) {
  if (currentBed) {
    scene.remove(currentBed);
    currentBed.geometry.dispose();
    currentBed.material.dispose();
    currentBed = null;
  }

  const size     = box.getSize(new THREE.Vector3());
  const center   = box.getCenter(new THREE.Vector3());
  const bedW     = size.x * 1.4;
  const bedH     = size.y * 1.4;
  const maxSide  = Math.max(bedW, bedH);
  const divisions = Math.min(32, Math.max(8,
    Math.round(maxSide / Math.min(size.x, size.y) * 8)
  ));

  const geo = new THREE.PlaneGeometry(bedW, bedH, divisions, divisions);
  const mat = new THREE.MeshBasicMaterial({
    color: 0x999690, wireframe: true, transparent: true, opacity: 0.35,
  });
  currentBed = new THREE.Mesh(geo, mat);
  currentBed.position.set(center.x, center.y, box.min.z);
  scene.add(currentBed);
}

// ── Load mesh ───────────────────────────────────────────────────────────────
function clearMesh() {
  if (currentMesh) {
    scene.remove(currentMesh);
    currentMesh.traverse(obj => {
      if (obj.isMesh) {
        obj.geometry.dispose();
        obj.material.dispose();
      }
    });
    currentMesh = null;
  }
  if (currentBed) {
    scene.remove(currentBed);
    currentBed.geometry.dispose();
    currentBed.material.dispose();
    currentBed = null;
  }
}

function loadMesh(model) {
  const placeholder = document.getElementById('viewer-placeholder');
  const loading     = document.getElementById('viewer-loading');

  clearMesh();
  placeholder.style.display = 'none';

  if (model.format !== 'stl' && model.format !== 'obj') {
    placeholder.querySelector('p').textContent =
      `Format "${model.format.toUpperCase()}" is not previewable — only STL and OBJ are supported.`;
    placeholder.style.display = 'flex';
    return;
  }

  loading.style.display = 'flex';

  fetchModelBuffer(model)
    .then(buffer => {
      loading.style.display = 'none';

      const makeMaterial = () => new THREE.MeshPhongMaterial({
        color: 0x7090c0, specular: 0x334466, shininess: 55,
        wireframe: isWireframe,
      });

      if (model.format === 'obj') {
        const text   = new TextDecoder().decode(buffer);
        const object = new THREE.OBJLoader().parse(text);
        // Keep only meshes with real triangle data — a NURBS/curve-only OBJ
        // (e.g. Rhino exports) yields empty geometries that render as NaN junk.
        const meshes = [];
        object.traverse(obj => {
          if (obj.isMesh && obj.geometry.attributes.position &&
              obj.geometry.attributes.position.count > 0) {
            meshes.push(obj);
          }
        });
        if (meshes.length === 0) {
          placeholder.querySelector('p').textContent =
            'This OBJ contains no triangle faces (curve/surface data only) — cannot preview.';
          placeholder.style.display = 'flex';
          return;
        }
        const group = new THREE.Group();
        for (const obj of meshes) {
          if (!obj.geometry.attributes.normal) obj.geometry.computeVertexNormals();
          obj.material = makeMaterial();
          group.add(obj);
        }
        currentMesh = group;
      } else {
        const geometry = new THREE.STLLoader().parse(fixTruncatedBinaryStl(buffer));
        geometry.computeVertexNormals();
        currentMesh = new THREE.Mesh(geometry, makeMaterial());
      }

      scene.add(currentMesh);
      fitCamera(new THREE.Box3().setFromObject(currentMesh));
    })
    .catch(err => {
      loading.style.display = 'none';
      placeholder.querySelector('p').textContent = 'Failed to load mesh.';
      placeholder.style.display = 'flex';
      console.error('Mesh load error:', err);
    });
}

// ── Download mesh ───────────────────────────────────────────────────────────
async function downloadMesh(model) {
  const btn = document.getElementById('btn-dl');
  if (btn) { btn.textContent = 'Downloading…'; btn.disabled = true; }

  try {
    const buffer = await fetchModelBuffer(model);
    const blob   = new Blob([buffer]);
    const url    = URL.createObjectURL(blob);
    const a      = document.createElement('a');
    a.href       = url;
    a.download   = `${model.id}.${model.format}`;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
  } catch (err) {
    alert('Download failed: ' + err.message);
  } finally {
    if (btn) { btn.textContent = `↓ Download ${model.format.toUpperCase()}`; btn.disabled = false; }
  }
}

// ── Details panel ───────────────────────────────────────────────────────────
function badge(label, value) {
  const cls = value ? 'badge-yes' : 'badge-no';
  const txt = value ? 'Yes' : 'No';
  return `<span class="badge ${cls}">${label}: ${txt}</span>`;
}

function showDetails(m) {
  currentDetailModel = m;
  document.getElementById('details').innerHTML = `
    <div class="details-header">
      <span class="details-name">${m.name}</span>
      <span class="details-id">ID ${m.id} &nbsp;·&nbsp; ${m.format.toUpperCase()}</span>
    </div>
    <div class="details-meta">
      <span>
        <strong>Thing:</strong>
        <a href="https://www.thingiverse.com/thing:${m.thing_id}" target="_blank">
          ${m.thing_id}
        </a>
      </span>
      <span><strong>License:</strong> ${m.license}</span>
      <button class="btn-download" id="btn-dl">↓ Download ${m.format.toUpperCase()}</button>
    </div>
    ${(m.vertices > 0 || m.faces > 0) ? `
    <div class="geo-stats">
      ${m.faces    > 0 ? `<span class="geo-stat">Faces<strong>${m.faces.toLocaleString()}</strong></span>`    : ''}
      ${m.vertices > 0 ? `<span class="geo-stat">Vertices<strong>${m.vertices.toLocaleString()}</strong></span>` : ''}
    </div>` : ''}
    <div class="badges">
      ${badge('Closed',            m.closed)}
      ${badge('Edge manifold',     m.edge_manifold)}
      ${badge('Vertex manifold',   m.vertex_manifold)}
      ${badge('Single component',  m.single_component)}
      ${badge('PWN',               m.pwn)}
      ${badge('Duplicated faces',  m.duplicated_faces)}
      ${badge('Degenerate faces',  m.degenerate_faces)}
    </div>
  `;
  document.getElementById('btn-dl').addEventListener('click', () => downloadMesh(m));
}

// ── Model list ──────────────────────────────────────────────────────────────
function buildList({ total, models }) {
  const list  = document.getElementById('model-list');
  const count = document.getElementById('model-count');

  const shown = models.length;
  count.textContent = shown === total
    ? `${total} model${total === 1 ? '' : 's'}`
    : `${shown} of ${total} models`;

  list.innerHTML = '';
  models.forEach(m => {
    const el = document.createElement('div');
    el.className = 'model-item' + (m.id === selectedId ? ' selected' : '');
    el.dataset.id = m.id;
    el.innerHTML = `
      <div class="item-name">${m.name}</div>
      <div class="item-meta">
        <span class="meta-badge">Thing ${m.thing_id}</span>
        ${m.faces > 0 ? `<span class="meta-badge">${m.faces.toLocaleString()} poly</span>` : ''}
        <span class="meta-badge meta-fmt">${m.format.toUpperCase()}</span>
      </div>
    `;
    el.addEventListener('click', () => selectModel(m));
    list.appendChild(el);
  });
}

function selectModel(m) {
  selectedId = m.id;
  localStorage.setItem('selectedModel', JSON.stringify(m));
  document.querySelectorAll('.model-item').forEach(el =>
    el.classList.toggle('selected', parseInt(el.dataset.id) === m.id)
  );
  showDetails(m);
  loadMesh(m);
  syncUrlToSelection(m.id);
}

// ── Polygon dual-range slider (custom drag) ───────────────────────────────────
function updatePolyFill() {
  const range   = polyAbsMax - polyAbsMin || 1;
  const lowPct  = ((polyMin - polyAbsMin) / range * 100).toFixed(2) + '%';
  const highPct = ((polyMax - polyAbsMin) / range * 100).toFixed(2) + '%';
  document.getElementById('poly-thumb-min').style.left = lowPct;
  document.getElementById('poly-thumb-max').style.left = highPct;
  document.getElementById('range-fill').style.left  = lowPct;
  document.getElementById('range-fill').style.width =
    ((polyMax - polyMin) / range * 100).toFixed(2) + '%';
  document.getElementById('poly-label-min').textContent = polyMin.toLocaleString();
  document.getElementById('poly-label-max').textContent = polyMax.toLocaleString();
}

function resetPolyToSet(models) {
  const faceCounts = models.map(m => m.faces).filter(f => f > 0);
  if (faceCounts.length === 0) return;
  let newMin = faceCounts[0], newMax = faceCounts[0];
  for (const f of faceCounts) {
    if (f < newMin) newMin = f;
    if (f > newMax) newMax = f;
  }
  polyAbsMin = newMin;
  polyAbsMax = newMax;
  polyMin    = newMin;
  polyMax    = newMax;
  updatePolyFill();
}

// ── Client-side filtering & sorting ─────────────────────────────────────────
function filterAndSort(skipPoly = false) {
  const search = document.getElementById('search').value.trim().toLowerCase();

  let results = allModels.filter(m => {
    // Text search
    if (search) {
      if (!String(m.id).includes(search)
          && !String(m.thing_id).includes(search)
          && !m.name.toLowerCase().includes(search)) return false;
    }
    // Boolean tri-state filters
    for (const [key, val] of Object.entries(filterState)) {
      if (val !== 'both' && m[key] !== (val === 'true')) return false;
    }
    // Polygon range filter
    if (!skipPoly && m.faces > 0) {
      if (m.faces < polyMin || m.faces > polyMax) return false;
    }
    return true;
  });

  // Sort
  switch (`${sortKey}_${sortDir}`) {
    case 'id_desc':    results.sort((a, b) => b.id - a.id); break;
    case 'thing_asc':  results.sort((a, b) => a.thing_id - b.thing_id); break;
    case 'thing_desc': results.sort((a, b) => b.thing_id - a.thing_id); break;
    case 'name_asc':   results.sort((a, b) => a.name.localeCompare(b.name)); break;
    case 'name_desc':  results.sort((a, b) => b.name.localeCompare(a.name)); break;
    default:           results.sort((a, b) => a.id - b.id); break;
  }

  return results;
}

function applyFilters({ resetPoly = false } = {}) {
  saveUIState();
  if (resetPoly) {
    const noPolyResults = filterAndSort(true);
    resetPolyToSet(noPolyResults);
    buildList({ total: noPolyResults.length, models: noPolyResults.slice(0, 100) });
  } else {
    const results = filterAndSort(false);
    buildList({ total: results.length, models: results.slice(0, 100) });
  }
}

// ── Sort button rendering ────────────────────────────────────────────────────
function renderSortButtons() {
  document.querySelectorAll('.sort-btn').forEach(btn => {
    const key    = btn.dataset.key;
    const active = key === sortKey;
    btn.classList.toggle('active', active);
    btn.querySelector('.arrow').textContent = active
      ? (sortDir === 'asc' ? '↑' : '↓')
      : '';
  });
}

// ── Tri-state filter helpers ─────────────────────────────────────────────────
function applyTriState(filter, value) {
  filterState[filter] = value;
  const group = document.querySelector(`.tri-group[data-filter="${filter}"]`);
  group.querySelectorAll('.tri-btn').forEach(btn => {
    btn.classList.toggle('active', btn.dataset.value === value);
  });
}

// ── UI state persistence ─────────────────────────────────────────────────────
function saveUIState() {
  localStorage.setItem('uiState', JSON.stringify({
    search:       document.getElementById('search').value,
    sortKey,
    sortDir,
    filters:      { ...filterState },
    polyMin,
    polyMax,
    filtersOpen:  document.getElementById('filters-section').open,
    polygonsOpen: document.getElementById('geo-section').open,
  }));
}

function restoreUIState() {
  try {
    const state = JSON.parse(localStorage.getItem('uiState'));
    if (!state) return;
    if (state.search)  document.getElementById('search').value = state.search;
    if (state.sortKey) sortKey = state.sortKey;
    if (state.sortDir) sortDir = state.sortDir;
    if (state.filters) {
      for (const [key, val] of Object.entries(state.filters)) {
        if (key in filterState && ['both', 'true', 'false'].includes(val)) {
          applyTriState(key, val);
        }
      }
    }
    // Restore poly range only if within current bounds
    if (state.polyMin != null && state.polyMax != null
        && state.polyMin >= polyAbsMin && state.polyMax <= polyAbsMax) {
      polyMin = state.polyMin;
      polyMax = state.polyMax;
      updatePolyFill();
    }
    if (state.filtersOpen  != null) document.getElementById('filters-section').open = state.filtersOpen;
    if (state.polygonsOpen != null) document.getElementById('geo-section').open     = state.polygonsOpen;
  } catch (_) { /* ignore corrupt state */ }
}

// ── Boot ────────────────────────────────────────────────────────────────────
initThree();

document.getElementById('btn-wireframe').addEventListener('click', () => {
  isWireframe = !isWireframe;
  document.getElementById('btn-wireframe').classList.toggle('active', isWireframe);
  if (currentMesh) {
    currentMesh.traverse(obj => {
      if (obj.isMesh) obj.material.wireframe = isWireframe;
    });
  }
});

// Tri-state filter buttons — reset poly range when these change
document.querySelectorAll('.tri-group').forEach(group => {
  group.addEventListener('click', e => {
    const btn = e.target.closest('.tri-btn');
    if (!btn) return;
    applyTriState(group.dataset.filter, btn.dataset.value);
    applyFilters({ resetPoly: true });
  });
});

// Sort buttons
document.querySelectorAll('.sort-btn').forEach(btn => {
  btn.addEventListener('click', () => {
    if (btn.dataset.key === sortKey) {
      sortDir = sortDir === 'asc' ? 'desc' : 'asc';
    } else {
      sortKey = btn.dataset.key;
      sortDir = 'asc';
    }
    renderSortButtons();
    applyFilters();
  });
});

// Search (debounced) — reset poly range when search changes
let debounce;
document.getElementById('search').addEventListener('input', () => {
  clearTimeout(debounce);
  debounce = setTimeout(() => applyFilters({ resetPoly: true }), 250);
});

// Polygon dual-range — custom drag so handles never block each other.
// When handles overlap, first mouse movement decides which thumb moves:
// drag LEFT → min, drag RIGHT → max.
(function initPolyDrag() {
  const track = document.getElementById('dual-range');

  function pctToValue(pct) {
    return Math.round(polyAbsMin + pct * (polyAbsMax - polyAbsMin));
  }

  track.addEventListener('mousedown', startDrag);
  track.addEventListener('touchstart', e => startDrag(e.touches[0]), { passive: true });

  function startDrag(e) {
    const rect  = track.getBoundingClientRect();
    const startX = e.clientX;
    const range  = polyAbsMax - polyAbsMin || 1;
    const minPct = (polyMin - polyAbsMin) / range;
    const maxPct = (polyMax - polyAbsMin) / range;
    const clickPct = Math.max(0, Math.min(1, (startX - rect.left) / rect.width));

    const distMin = Math.abs(clickPct - minPct);
    const distMax = Math.abs(clickPct - maxPct);

    // If handles are not clearly separated, defer thumb choice to first drag direction
    let isMin = distMin <= distMax;
    let decided = Math.abs(distMin - distMax) > 0.01;

    const minThumb = document.getElementById('poly-thumb-min');
    const maxThumb = document.getElementById('poly-thumb-max');

    function onMove(e) {
      const clientX = e.touches ? e.touches[0].clientX : e.clientX;
      if (!decided) {
        const dx = clientX - startX;
        if (Math.abs(dx) < 3) return;    // wait for clear direction
        isMin   = dx < 0;                // left = move min, right = move max
        decided = true;
      }

      const pct = Math.max(0, Math.min(1, (clientX - rect.left) / rect.width));
      const val = pctToValue(pct);

      if (isMin) {
        polyMin = Math.max(polyAbsMin, Math.min(val, polyMax - 1));
        minThumb.classList.add('dragging');
        maxThumb.classList.remove('dragging');
      } else {
        polyMax = Math.min(polyAbsMax, Math.max(val, polyMin + 1));
        maxThumb.classList.add('dragging');
        minThumb.classList.remove('dragging');
      }

      updatePolyFill();
      clearTimeout(debounce);
      debounce = setTimeout(() => applyFilters(), 120);
    }

    function onEnd() {
      minThumb.classList.remove('dragging');
      maxThumb.classList.remove('dragging');
      document.removeEventListener('mousemove', onMove);
      document.removeEventListener('mouseup',   onEnd);
      document.removeEventListener('touchmove', onMove);
      document.removeEventListener('touchend',  onEnd);
    }

    document.addEventListener('mousemove', onMove);
    document.addEventListener('mouseup',   onEnd);
    document.addEventListener('touchmove', onMove, { passive: false });
    document.addEventListener('touchend',  onEnd);
  }
}());

// Load all model metadata, then boot the UI
fetch('data/models.json')
  .then(r => r.json())
  .then(models => {
    allModels = models;

    // Init polygon slider from full dataset
    const geoSection = document.getElementById('geo-section');
    geoSection.removeAttribute('hidden');
    geoSection.open = true;  // default open
    resetPolyToSet(models);

    restoreUIState();
    renderSortButtons();
    applyFilters({ resetPoly: true });

    // Attach accordion listeners AFTER restoreUIState so programmatic open
    // during init doesn't overwrite the saved state
    document.getElementById('filters-section').addEventListener('toggle', saveUIState);
    document.getElementById('geo-section').addEventListener('toggle',     saveUIState);

    // A `?model=` deep link outranks the restored selection. It also outranks
    // the restored filters: if they hide the model from the list we still load
    // it in the viewer — only the list scroll is best-effort.
    const linked = DEEP_LINK_ID != null
      ? allModels.find(m => m.id === DEEP_LINK_ID)
      : null;
    if (linked) {
      selectModel(linked);
      scrollListEntryIntoView(linked.id);
      return;
    }

    // Restore previously selected model
    try {
      const saved = JSON.parse(localStorage.getItem('selectedModel'));
      if (saved) {
        // Find the live record so repo field etc. are current. A model that
        // no longer exists (removed from the dataset) must not be restored
        // from the stale localStorage copy — its mesh file is gone too.
        const live = allModels.find(m => m.id === saved.id);
        if (live) {
          selectedId = live.id;
          document.querySelectorAll('.model-item').forEach(el =>
            el.classList.toggle('selected', parseInt(el.dataset.id) === live.id)
          );
          showDetails(live);
          loadMesh(live);
        } else {
          localStorage.removeItem('selectedModel');
        }
      }
    } catch (_) { /* ignore corrupt state */ }
  })
  .catch(err => {
    document.getElementById('model-count').textContent = 'Failed to load data.';
    console.error('Failed to load models.json:', err);
  });
