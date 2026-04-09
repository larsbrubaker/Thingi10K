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

const GEO_INPUT_IDS = [
  'f-min-faces', 'f-max-faces', 'f-min-vertices', 'f-max-vertices',
];

// ── Mesh CDN ────────────────────────────────────────────────────────────────
const MESH_CDN = 'https://cdn.jsdelivr.net/gh/larsbrubaker';

function meshZipUrl(model) {
  return `${MESH_CDN}/Thingi10K-meshes-${model.repo}@main/meshes/${model.id}.${model.format}.zip`;
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
      resolve(data.buffer);
    });
  });
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
function fitCamera(geometry) {
  geometry.computeBoundingBox();
  const box    = geometry.boundingBox;
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
    currentMesh.geometry.dispose();
    currentMesh.material.dispose();
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

  if (model.format !== 'stl') {
    placeholder.querySelector('p').textContent =
      `Format "${model.format.toUpperCase()}" is not previewable — only STL is supported.`;
    placeholder.style.display = 'flex';
    return;
  }

  loading.style.display = 'flex';

  fetchAndDecompress(meshZipUrl(model))
    .then(buffer => {
      loading.style.display = 'none';
      const loader   = new THREE.STLLoader();
      const geometry = loader.parse(buffer);
      geometry.computeVertexNormals();

      const mat = new THREE.MeshPhongMaterial({
        color: 0x7090c0, specular: 0x334466, shininess: 55,
      });
      currentMesh = new THREE.Mesh(geometry, mat);
      currentMesh.material.wireframe = isWireframe;
      scene.add(currentMesh);
      fitCamera(geometry);
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
    const buffer = await fetchAndDecompress(meshZipUrl(model));
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
        <span>${m.id}</span>
        <span>·</span>
        <span>Thing ${m.thing_id}</span>
        <span class="fmt-badge">${m.format}</span>
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
}

// ── Client-side filtering & sorting ─────────────────────────────────────────
function filterAndSort() {
  const search  = document.getElementById('search').value.trim().toLowerCase();
  const minFaces = parseFloat(document.getElementById('f-min-faces').value);
  const maxFaces = parseFloat(document.getElementById('f-max-faces').value);
  const minVerts = parseFloat(document.getElementById('f-min-vertices').value);
  const maxVerts = parseFloat(document.getElementById('f-max-vertices').value);

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
    // Geometry range filters
    if (m.faces > 0) {
      if (!isNaN(minFaces) && m.faces < minFaces) return false;
      if (!isNaN(maxFaces) && m.faces > maxFaces) return false;
    }
    if (m.vertices > 0) {
      if (!isNaN(minVerts) && m.vertices < minVerts) return false;
      if (!isNaN(maxVerts) && m.vertices > maxVerts) return false;
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

function applyFilters() {
  saveUIState();
  const all = filterAndSort();
  buildList({ total: all.length, models: all.slice(0, 100) });
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
    search:  document.getElementById('search').value,
    sortKey,
    sortDir,
    filters: { ...filterState },
    geo: Object.fromEntries(
      GEO_INPUT_IDS.map(id => [id, document.getElementById(id).value])
    ),
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
    GEO_INPUT_IDS.forEach(id => {
      const v = state.geo?.[id];
      if (v != null && v !== '') document.getElementById(id).value = v;
    });
  } catch (_) { /* ignore corrupt state */ }
}

// ── Boot ────────────────────────────────────────────────────────────────────
initThree();

document.getElementById('btn-wireframe').addEventListener('click', () => {
  isWireframe = !isWireframe;
  document.getElementById('btn-wireframe').classList.toggle('active', isWireframe);
  if (currentMesh) currentMesh.material.wireframe = isWireframe;
});

// Tri-state filter buttons
document.querySelectorAll('.tri-group').forEach(group => {
  group.addEventListener('click', e => {
    const btn = e.target.closest('.tri-btn');
    if (!btn) return;
    applyTriState(group.dataset.filter, btn.dataset.value);
    applyFilters();
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

// Search (debounced)
let debounce;
document.getElementById('search').addEventListener('input', () => {
  clearTimeout(debounce);
  debounce = setTimeout(applyFilters, 250);
});

// Geometry range inputs (debounced)
GEO_INPUT_IDS.forEach(id => {
  document.getElementById(id).addEventListener('input', () => {
    clearTimeout(debounce);
    debounce = setTimeout(applyFilters, 400);
  });
});

// Load all model metadata, then boot the UI
fetch('data/models.json')
  .then(r => r.json())
  .then(models => {
    allModels = models;

    // Set geo filter max attributes from dataset
    const maxV = Math.max(...models.map(m => m.vertices));
    const maxF = Math.max(...models.map(m => m.faces));
    const geoSection = document.getElementById('geo-section');
    geoSection.removeAttribute('hidden');
    geoSection.open = false;  // collapsed by default
    document.getElementById('f-max-faces').setAttribute('max', maxF);
    document.getElementById('f-min-faces').setAttribute('max', maxF);
    document.getElementById('f-max-vertices').setAttribute('max', maxV);
    document.getElementById('f-min-vertices').setAttribute('max', maxV);

    restoreUIState();
    renderSortButtons();
    applyFilters();

    // Restore previously selected model
    try {
      const saved = JSON.parse(localStorage.getItem('selectedModel'));
      if (saved) {
        // Find the live record so repo field etc. are current
        const live = allModels.find(m => m.id === saved.id) || saved;
        selectedId = live.id;
        document.querySelectorAll('.model-item').forEach(el =>
          el.classList.toggle('selected', parseInt(el.dataset.id) === live.id)
        );
        showDetails(live);
        loadMesh(live);
      }
    } catch (_) { /* ignore corrupt state */ }
  })
  .catch(err => {
    document.getElementById('model-count').textContent = 'Failed to load data.';
    console.error('Failed to load models.json:', err);
  });
