'use strict';

// ── State ───────────────────────────────────────────────────────────────────
let scene, camera, renderer, controls, currentMesh, currentBed;
let selectedId  = null;
let isWireframe = false;

// Sort state: key is 'id' | 'thing' | 'name', dir is 'asc' | 'desc'.
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
  // STL files use Z-up; orient the camera accordingly.
  camera.up.set(0, 0, 1);
  camera.position.set(0, -100, 50);

  renderer = new THREE.WebGLRenderer({ antialias: true });
  renderer.setPixelRatio(window.devicePixelRatio);
  renderer.setSize(container.clientWidth, container.clientHeight);
  container.appendChild(renderer.domElement);

  // Lighting tuned for the warm light theme viewer background.
  scene.add(new THREE.AmbientLight(0xffffff, 0.6));

  const key = new THREE.DirectionalLight(0xffffff, 0.85);
  key.position.set(2, -3, 4);
  scene.add(key);

  const fill = new THREE.DirectionalLight(0xfff8f0, 0.3);
  fill.position.set(-2, 1, -3);
  scene.add(fill);

  controls = new THREE.OrbitControls(camera, renderer.domElement);
  controls.enableDamping  = true;
  controls.dampingFactor  = 0.06;

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

  // Front-side-above angle; natural for Z-up STL models.
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

// ── Build plate (wireframe bed) ─────────────────────────────────────────────
function addBed(box) {
  if (currentBed) {
    scene.remove(currentBed);
    currentBed.geometry.dispose();
    currentBed.material.dispose();
    currentBed = null;
  }

  const size   = box.getSize(new THREE.Vector3());
  const center = box.getCenter(new THREE.Vector3());

  const bedW     = size.x * 1.4;
  const bedH     = size.y * 1.4;
  const maxSide  = Math.max(bedW, bedH);
  const divisions = Math.min(32, Math.max(8,
    Math.round(maxSide / Math.min(size.x, size.y) * 8)
  ));

  // PlaneGeometry lies in XY plane — correct for Z-up.
  const geo = new THREE.PlaneGeometry(bedW, bedH, divisions, divisions);
  const mat = new THREE.MeshBasicMaterial({
    color: 0x999690,
    wireframe: true,
    transparent: true,
    opacity: 0.35,
  });
  currentBed = new THREE.Mesh(geo, mat);
  currentBed.position.set(center.x, center.y, box.min.z);
  scene.add(currentBed);
}

// ── Load mesh ───────────────────────────────────────────────────────────────
function loadMesh(id, fmt) {
  const placeholder = document.getElementById('viewer-placeholder');
  const loading     = document.getElementById('viewer-loading');

  // Clear existing mesh and bed.
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

  placeholder.style.display = 'none';

  if (fmt !== 'stl') {
    placeholder.querySelector('p').textContent =
      `Format "${fmt.toUpperCase()}" is not previewable — only STL is supported.`;
    placeholder.style.display = 'flex';
    return;
  }

  loading.style.display = 'flex';

  const loader = new THREE.STLLoader();
  loader.load(
    `/mesh/${id}`,
    (geometry) => {
      loading.style.display = 'none';
      geometry.computeVertexNormals();

      const mat = new THREE.MeshPhongMaterial({
        color:     0x7090c0,
        specular:  0x334466,
        shininess: 55,
      });
      currentMesh = new THREE.Mesh(geometry, mat);
      currentMesh.material.wireframe = isWireframe;
      scene.add(currentMesh);
      fitCamera(geometry);
    },
    undefined,
    (err) => {
      loading.style.display = 'none';
      placeholder.querySelector('p').textContent = 'Failed to load mesh.';
      placeholder.style.display = 'flex';
      console.error('STL load error:', err);
    }
  );
}

// ── Details panel ───────────────────────────────────────────────────────────
function badge(label, value) {
  const cls = value ? 'badge-yes' : 'badge-no';
  const txt = value ? 'Yes' : 'No';
  return `<span class="badge ${cls}">${label}: ${txt}</span>`;
}

function showDetails(m) {
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
      <a class="btn-download" href="/mesh/${m.id}">
        ↓ Download ${m.format.toUpperCase()}
      </a>
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
  loadMesh(m.id, m.format);
}

// ── Filtering ───────────────────────────────────────────────────────────────
// ── Sort button rendering ───────────────────────────────────────────────────
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

function buildParams() {
  const params = new URLSearchParams();
  const search = document.getElementById('search').value.trim();

  if (search) params.set('search', search);
  params.set('sort', `${sortKey}_${sortDir}`);

  for (const [key, val] of Object.entries(filterState)) {
    if (val !== 'both') params.set(key, val);
  }

  // Geometry range filters — only send when the user has entered a value.
  const minFaces = document.getElementById('f-min-faces').value;
  const maxFaces = document.getElementById('f-max-faces').value;
  const minVerts = document.getElementById('f-min-vertices').value;
  const maxVerts = document.getElementById('f-max-vertices').value;
  if (minFaces) params.set('min_faces', minFaces);
  if (maxFaces) params.set('max_faces', maxFaces);
  if (minVerts) params.set('min_vertices', minVerts);
  if (maxVerts) params.set('max_vertices', maxVerts);

  return params;
}

function applyTriState(filter, value) {
  filterState[filter] = value;
  const group = document.querySelector(`.tri-group[data-filter="${filter}"]`);
  group.querySelectorAll('.tri-btn').forEach(btn => {
    btn.classList.toggle('active', btn.dataset.value === value);
  });
}

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

function applyFilters() {
  saveUIState();
  fetch(`/api/models?${buildParams()}`)
    .then(r => r.json())
    .then(buildList)
    .catch(console.error);
}

// ── Boot ────────────────────────────────────────────────────────────────────
initThree();

// Wireframe toggle.
document.getElementById('btn-wireframe').addEventListener('click', () => {
  isWireframe = !isWireframe;
  document.getElementById('btn-wireframe').classList.toggle('active', isWireframe);
  if (currentMesh) currentMesh.material.wireframe = isWireframe;
});

// Restore saved search/filter/sort state, then fetch the list.
restoreUIState();
renderSortButtons();

// Fetch geometry stats — show/hide the Geometry section accordingly.
fetch('/api/stats')
  .then(r => r.json())
  .then(stats => {
    const section = document.getElementById('geo-section');
    if (stats.has_geometry) {
      section.removeAttribute('hidden');
      // Set sensible max attributes on the inputs.
      document.getElementById('f-max-faces').setAttribute('max', stats.max_faces);
      document.getElementById('f-min-faces').setAttribute('max', stats.max_faces);
      document.getElementById('f-max-vertices').setAttribute('max', stats.max_vertices);
      document.getElementById('f-min-vertices').setAttribute('max', stats.max_vertices);
    }
  })
  .catch(() => { /* stats unavailable — keep section hidden */ });

fetch(`/api/models?${buildParams()}`)
  .then(r => r.json())
  .then(data => {
    buildList(data);

    // Restore previously selected model.
    try {
      const saved = JSON.parse(localStorage.getItem('selectedModel'));
      if (saved) {
        selectedId = saved.id;
        document.querySelectorAll('.model-item').forEach(el =>
          el.classList.toggle('selected', parseInt(el.dataset.id) === saved.id)
        );
        showDetails(saved);
        loadMesh(saved.id, saved.format);
      }
    } catch (_) { /* ignore corrupt state */ }
  })
  .catch(console.error);

// Search (debounced) and filter/sort listeners.
let debounce;
document.getElementById('search').addEventListener('input', () => {
  clearTimeout(debounce);
  debounce = setTimeout(applyFilters, 250);
});

document.querySelectorAll('.tri-group').forEach(group => {
  group.addEventListener('click', e => {
    const btn = e.target.closest('.tri-btn');
    if (!btn) return;
    applyTriState(group.dataset.filter, btn.dataset.value);
    applyFilters();
  });
});

// Geometry range inputs — debounced so rapid typing doesn't hammer the server.
GEO_INPUT_IDS.forEach(id => {
  document.getElementById(id).addEventListener('input', () => {
    clearTimeout(debounce);
    debounce = setTimeout(applyFilters, 400);
  });
});

// Sort buttons — click active button to flip direction; click inactive to switch.
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
