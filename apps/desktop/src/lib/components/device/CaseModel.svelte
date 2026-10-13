<script lang="ts">
  // The Raze, as its case: the case model (static/models/raze-case.glb, built
  // by devices/tools/case_model.py from the CAD: the case, a simplified board
  // and the connectors at its edges, in the case's frame) turned by the
  // case's orientation (imu.ts caseOrientation), with the lens added and the axes drawn. three.js
  // loads only when this shows. `onfail` says WebGL or the model isn't there,
  // so the caller can draw something simpler. With `ring`, the status ring's
  // LEDs glow as the board's do (lemnosd's frames, see ring.ts).
  import { onMount } from "svelte";
  import type { Quat } from "./imu.ts";
  import type { RingFrame } from "./ring.ts";

  let {
    orientation,
    onfail,
    ring = () => null,
  }: { orientation: () => Quat; onfail: () => void; ring?: () => RingFrame | null } = $props();

  let host = $state<HTMLDivElement>();

  onMount(() => {
    let stop = () => {};
    let gone = false;
    void (async () => {
      try {
        const THREE = await import("three");
        const { GLTFLoader } = await import("three/examples/jsm/loaders/GLTFLoader.js");
        const { RoomEnvironment } = await import("three/examples/jsm/environments/RoomEnvironment.js");
        const { toCreasedNormals } = await import("three/examples/jsm/utils/BufferGeometryUtils.js");
        const { OrbitControls } = await import("three/examples/jsm/controls/OrbitControls.js");
        if (gone || !host) return;
        const renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true });
        renderer.setPixelRatio(Math.min(2, window.devicePixelRatio || 1));
        renderer.toneMapping = THREE.ACESFilmicToneMapping;
        renderer.toneMappingExposure = 1.05;
        // A real shadow under and across the case: most of what makes its shape read.
        renderer.shadowMap.enabled = true;
        renderer.shadowMap.type = THREE.PCFSoftShadowMap;
        host.appendChild(renderer.domElement);
        const scene = new THREE.Scene();
        // A soft room to reflect: the metal and the glass read as such, and
        // the dark plastic keeps its shape in the shadows.
        const pmrem = new THREE.PMREMGenerator(renderer);
        const environment = pmrem.fromScene(new RoomEnvironment(), 0.04).texture;
        scene.environment = environment;
        scene.environmentIntensity = 0.9;
        const camera = new THREE.PerspectiveCamera(32, 1, 0.005, 1);
        // From the front, a little above and to the side, as far off as the
        // case needs to fit the view's shape (set on resize).
        const VIEW_FROM = new THREE.Vector3(0.075, 0.07, 0.11).normalize();
        /** About the case's half size (m): it fits however it is turned. */
        const FIT_RADIUS = 0.04;
        scene.add(new THREE.HemisphereLight(0xe6ecf5, 0x15181d, 0.35));
        // Key from above and to one side, casting; a cool rim behind; a low
        // warm fill so the shadow side keeps its detail.
        const key = new THREE.DirectionalLight(0xffffff, 2.6);
        key.position.set(0.12, 0.3, 0.16);
        key.castShadow = true;
        key.shadow.mapSize.set(1024, 1024);
        key.shadow.camera.near = 0.05;
        key.shadow.camera.far = 0.8;
        key.shadow.camera.left = key.shadow.camera.bottom = -0.07;
        key.shadow.camera.right = key.shadow.camera.top = 0.07;
        key.shadow.bias = -0.0004;
        key.shadow.normalBias = 0.0008;
        key.shadow.radius = 4;
        scene.add(key);
        const rim = new THREE.DirectionalLight(0x9ab4ff, 1.4);
        rim.position.set(-0.5, 0.25, -0.45);
        scene.add(rim);
        const fill = new THREE.DirectionalLight(0xffe2c4, 0.45);
        fill.position.set(-0.4, -0.1, 0.4);
        scene.add(fill);
        // The floor it sits over: only its shadow shows, softly.
        const floor = new THREE.Mesh(new THREE.PlaneGeometry(0.4, 0.4), new THREE.ShadowMaterial({ opacity: 0.35 }));
        floor.rotation.x = -Math.PI / 2;
        floor.position.y = -0.042;
        floor.receiveShadow = true;
        scene.add(floor);

        // The board: the case, turned as the IMU is. Its frame is the IMU's
        // (z out of the front); the world's z is up, three's y is up.
        const board = new THREE.Group();
        const world = new THREE.Group();
        world.quaternion.setFromAxisAngle(new THREE.Vector3(1, 0, 0), -Math.PI / 2);
        world.add(board);
        scene.add(world);

        const style = getComputedStyle(document.documentElement);
        const css = (name: string, fallback: string) => style.getPropertyValue(name).trim() || fallback;
        const MATERIALS: Record<string, InstanceType<typeof THREE.MeshStandardMaterial>> = {
          // Printed plastic: a mid grey light enough to shade, a satin sheen.
          TOP: new THREE.MeshPhysicalMaterial({ color: 0x4a5059, roughness: 0.48, metalness: 0, clearcoat: 0.25, clearcoatRoughness: 0.5 }),
          BOTTOM: new THREE.MeshPhysicalMaterial({ color: 0x3a3f47, roughness: 0.55, metalness: 0, clearcoat: 0.15, clearcoatRoughness: 0.6 }),
          HEATSINK: new THREE.MeshStandardMaterial({ color: 0xb4bcc6, roughness: 0.28, metalness: 0.95 }),
          DIFFUSER: new THREE.MeshStandardMaterial({
            color: 0xf2f4f7,
            roughness: 0.4,
            transparent: true,
            opacity: 0.85,
            emissive: new THREE.Color(css("--accent", "#ff6b6b")),
            emissiveIntensity: 0.35,
          }),
          BUTTON: new THREE.MeshStandardMaterial({ color: 0x4a505a, roughness: 0.55 }),
        };
        const gltf = await new GLTFLoader().loadAsync("/models/raze-case.glb");
        gltf.scene.traverse((node) => {
          const mesh = node as InstanceType<typeof THREE.Mesh>;
          // CAD meshes are welded across hard edges: smooth normals only
          // where the surface really curves, sharp at the edges.
          if (mesh.isMesh) mesh.geometry = toCreasedNormals(mesh.geometry, Math.PI / 6);
          // The case's parts get these; the board's (PCB, IO_n) keep their own colours.
          const own = MATERIALS[mesh.name] ?? MATERIALS[mesh.parent?.name ?? ""];
          if (mesh.isMesh && own) mesh.material = own;
          else if (mesh.isMesh) {
            // The board's parts keep their materials from the model (each
            // connector coloured by what it is, see case_model.py).
            const loaded = mesh.material as InstanceType<typeof THREE.MeshStandardMaterial>;
            loaded.flatShading = false;
          }
        });
        board.add(gltf.scene);
        // Every part casts and takes shadow; the case's parts get a fine dark
        // edge where the surface turns sharply, so its shape reads at any angle.
        const edgeMaterial = new THREE.LineBasicMaterial({ color: 0x0b0d10, transparent: true, opacity: 0.55 });
        gltf.scene.traverse((node) => {
          const mesh = node as InstanceType<typeof THREE.Mesh>;
          if (!mesh.isMesh) return;
          mesh.castShadow = true;
          mesh.receiveShadow = true;
          const part = mesh.name || mesh.parent?.name || "";
          if (part === "TOP" || part === "BOTTOM") {
            mesh.add(new THREE.LineSegments(new THREE.EdgesGeometry(mesh.geometry, 35), edgeMaterial));
          }
        });
        // The lens, which the case model leaves out, in the ring's centre on
        // the front face: a metal barrel, coated glass that catches the room,
        // and the tinted iris behind it.
        const front = new THREE.Box3().setFromObject(gltf.scene).max.z;
        const lens = new THREE.Group();
        const barrel = new THREE.Mesh(
          new THREE.CylinderGeometry(0.0047, 0.0049, 0.004, 48, 1, true),
          new THREE.MeshStandardMaterial({ color: 0x5b616b, roughness: 0.35, metalness: 0.9, side: THREE.DoubleSide }),
        );
        const bezel = new THREE.Mesh(
          new THREE.TorusGeometry(0.0043, 0.00055, 12, 48),
          new THREE.MeshStandardMaterial({ color: 0x8b929c, roughness: 0.25, metalness: 0.95 }),
        );
        bezel.rotation.x = Math.PI / 2;
        bezel.position.y = 0.002;
        const iris = new THREE.Mesh(
          new THREE.CircleGeometry(0.0039, 48),
          new THREE.MeshStandardMaterial({ color: 0x1b2a48, roughness: 0.3, metalness: 0.5, emissive: 0x0a1430, emissiveIntensity: 0.6 }),
        );
        iris.rotation.x = -Math.PI / 2;
        iris.position.y = 0.0008;
        const pupil = new THREE.Mesh(
          new THREE.CircleGeometry(0.0017, 32),
          new THREE.MeshStandardMaterial({ color: 0x050608, roughness: 0.2 }),
        );
        pupil.rotation.x = -Math.PI / 2;
        pupil.position.y = 0.00085;
        const glass = new THREE.Mesh(
          new THREE.SphereGeometry(0.0042, 48, 12, 0, Math.PI * 2, 0, Math.PI / 5),
          new THREE.MeshPhysicalMaterial({
            color: 0x9fb4d8,
            roughness: 0.04,
            metalness: 0,
            transmission: 0,
            transparent: true,
            opacity: 0.28,
            clearcoat: 1,
            clearcoatRoughness: 0.03,
            envMapIntensity: 2.2,
          }),
        );
        glass.scale.y = 0.35;
        glass.position.y = 0.0006;
        lens.add(barrel, bezel, iris, pupil, glass);
        lens.rotation.x = Math.PI / 2;
        lens.position.set(0, 0, front - 0.0021);
        board.add(lens);
        // The ring's LEDs light the diffuser itself: each of its vertices
        // takes the colour of the LEDs nearest its angle around the lens
        // (blended between the two), over the plastic's own dim white.
        // Physical LED 0 sits at RING_LED0_DEG from +x toward +y, the rest
        // following that way, one step per 360/n degrees: the Raze's gravity
        // settings (gravity_plane ["x", "y"], gravity_led0_deg 112.5) as
        // lemnos-board's gravity.rs reads them, checked live on a board. A
        // board with other gravity axes would need them here.
        // Lemnos's angle is in the IMU's frame (112.5° from its +x toward +y);
        // the diffuser's vertices are in the case's (imu.ts CASE_IN_IMU, a half
        // turn that mirrors the ring's plane): LED 0 at 157.5° here, the rest
        // going the other way round.
        const RING_LED0_DEG = 157.5;
        const RING_DIRECTION = -1;
        const diffusers: InstanceType<typeof THREE.Mesh>[] = [];
        gltf.scene.traverse((node) => {
          const mesh = node as InstanceType<typeof THREE.Mesh>;
          if (mesh.isMesh && (mesh.name === "DIFFUSER" || mesh.parent?.name === "DIFFUSER")) diffusers.push(mesh);
        });
        const BASE = 0.16;
        /** Per diffuser vertex: its position on the ring, 0..1 from LED 0. */
        let around: Float32Array[] = [];
        let lit = false;
        const paint = (frame: RingFrame) => {
          const n = frame.colors.length;
          if (!n || !diffusers.length) return;
          if (!lit) {
            lit = true;
            around = diffusers.map((mesh) => {
              const position = mesh.geometry.getAttribute("position");
              const at = new Float32Array(position.count);
              for (let v = 0; v < position.count; v++) {
                const deg = (Math.atan2(position.getY(v), position.getX(v)) * 180) / Math.PI;
                at[v] = ((((deg - RING_LED0_DEG) * RING_DIRECTION) % 360) + 360) % 360 / 360;
              }
              mesh.geometry.setAttribute("color", new THREE.BufferAttribute(new Float32Array(position.count * 3), 3));
              mesh.material = new THREE.MeshBasicMaterial({ vertexColors: true, transparent: true, opacity: 0.95 });
              return at;
            });
          }
          const rgb = frame.colors.map((c) => [((c >> 16) & 0xff) / 255, ((c >> 8) & 0xff) / 255, (c & 0xff) / 255]);
          diffusers.forEach((mesh, m) => {
            const colors = mesh.geometry.getAttribute("color") as InstanceType<typeof THREE.BufferAttribute>;
            const at = around[m];
            for (let v = 0; v < at.length; v++) {
              const x = at[v] * n;
              const i0 = Math.floor(x) % n;
              const i1 = (i0 + 1) % n;
              const t = x - Math.floor(x);
              for (let k = 0; k < 3; k++) {
                // Lifted a little: a ring at low brightness still reads on screen.
                const led = rgb[i0][k] * (1 - t) + rgb[i1][k] * t;
                colors.array[v * 3 + k] = Math.min(1, BASE + led * 1.4);
              }
            }
            colors.needsUpdate = true;
          });
        };
        let lastFrame: RingFrame | null = null;

        // The case's axes: x red, y green, z blue (out of the lens).
        const axes = [
          [new THREE.Vector3(1, 0, 0), css("--err", "#fb7185")],
          [new THREE.Vector3(0, 1, 0), css("--ok", "#4ade80")],
          [new THREE.Vector3(0, 0, 1), css("--info", "#60a5fa")],
        ] as const;
        for (const [dir, color] of axes) {
          board.add(new THREE.ArrowHelper(dir, new THREE.Vector3(0, 0, 0), 0.05, new THREE.Color(color).getHex(), 0.008, 0.005));
        }

        // Drag to look around it, scroll to come closer, double-click to go
        // back to the view it opens with.
        const controls = new OrbitControls(camera, renderer.domElement);
        controls.enableDamping = true;
        controls.dampingFactor = 0.12;
        controls.enablePan = false;
        controls.rotateSpeed = 0.8;
        controls.target.set(0, 0, 0);
        /** The distance that fits the case in the view, from its shape. */
        let fit = 0.1;
        let moved = false;
        controls.addEventListener("start", () => (moved = true));
        const home = () => {
          moved = false;
          camera.position.copy(VIEW_FROM).multiplyScalar(fit);
          camera.lookAt(0, 0, 0);
          controls.update();
        };
        renderer.domElement.addEventListener("dblclick", home);
        renderer.domElement.style.cursor = "grab";

        const resize = () => {
          if (!host) return;
          const { clientWidth: w, clientHeight: h } = host;
          renderer.setSize(w, h, false);
          camera.aspect = w / Math.max(1, h);
          const vertical = (camera.fov * Math.PI) / 180;
          const horizontal = 2 * Math.atan(Math.tan(vertical / 2) * camera.aspect);
          const before = fit;
          fit = FIT_RADIUS / Math.sin(Math.min(vertical, horizontal) / 2);
          controls.minDistance = fit * 0.45;
          controls.maxDistance = fit * 3;
          // Turned by hand: keep the angle, scale the distance with the view.
          if (moved) camera.position.multiplyScalar(fit / before);
          else camera.position.copy(VIEW_FROM).multiplyScalar(fit);
          camera.lookAt(0, 0, 0);
          camera.updateProjectionMatrix();
          controls.update();
        };
        const observer = new ResizeObserver(resize);
        observer.observe(host);
        resize();

        const q = new THREE.Quaternion();
        let frame = 0;
        const draw = () => {
          const [w, x, y, z] = orientation();
          q.set(x, y, z, w);
          board.quaternion.copy(q);
          const now = ring();
          if (now && now !== lastFrame) {
            lastFrame = now;
            paint(now);
          }
          controls.update();
          renderer.render(scene, camera);
          frame = requestAnimationFrame(draw);
        };
        frame = requestAnimationFrame(draw);
        stop = () => {
          cancelAnimationFrame(frame);
          observer.disconnect();
          renderer.domElement.removeEventListener("dblclick", home);
          controls.dispose();
          environment.dispose();
          pmrem.dispose();
          renderer.dispose();
          renderer.domElement.remove();
        };
      } catch (error) {
        console.warn("case model:", error);
        if (!gone) onfail();
      }
    })();
    return () => {
      gone = true;
      stop();
    };
  });
</script>

<div class="model" bind:this={host}></div>

<style>
  /* The canvas fills the box without sizing it: the box is the layout's. */
  .model {
    position: relative;
    width: 100%;
    height: 100%;
    min-height: 150px;
  }
  .model :global(canvas) {
    position: absolute;
    inset: 0;
    display: block;
    touch-action: none;
    width: 100%;
    height: 100%;
  }
</style>
