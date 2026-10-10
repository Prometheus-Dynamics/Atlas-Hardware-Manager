<script lang="ts">
  // The Raze, as its case: the case model (static/models/raze-case.glb, built
  // by devices/tools/case_model.py from the CAD: the case, a simplified board
  // and the connectors at its edges, in the IMU's frame) turned
  // by the IMU's orientation, with the lens added and the axes drawn. three.js
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
        if (gone || !host) return;
        const renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true });
        renderer.setPixelRatio(Math.min(2, window.devicePixelRatio || 1));
        renderer.toneMapping = THREE.ACESFilmicToneMapping;
        renderer.toneMappingExposure = 1.1;
        host.appendChild(renderer.domElement);
        const scene = new THREE.Scene();
        // A soft room to reflect: the metal and the glass read as such, and
        // the dark plastic keeps its shape in the shadows.
        const pmrem = new THREE.PMREMGenerator(renderer);
        const environment = pmrem.fromScene(new RoomEnvironment(), 0.04).texture;
        scene.environment = environment;
        scene.environmentIntensity = 0.55;
        const camera = new THREE.PerspectiveCamera(32, 1, 0.005, 1);
        // From the front, a little above and to the side, as far off as the
        // case needs to fit the view's shape (set on resize).
        const VIEW_FROM = new THREE.Vector3(0.075, 0.07, 0.11).normalize();
        /** About the case's half size (m): it fits however it is turned. */
        const FIT_RADIUS = 0.04;
        scene.add(new THREE.HemisphereLight(0xdfe6f2, 0x20242c, 0.5));
        const key = new THREE.DirectionalLight(0xffffff, 1.8);
        key.position.set(0.3, 0.6, 0.5);
        scene.add(key);
        const rim = new THREE.DirectionalLight(0x9ab4ff, 0.8);
        rim.position.set(-0.5, 0.2, -0.4);
        scene.add(rim);

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
          TOP: new THREE.MeshStandardMaterial({ color: 0x33373e, roughness: 0.62, metalness: 0 }),
          BOTTOM: new THREE.MeshStandardMaterial({ color: 0x2a2d33, roughness: 0.68, metalness: 0 }),
          HEATSINK: new THREE.MeshStandardMaterial({ color: 0xa4acb6, roughness: 0.32, metalness: 0.9 }),
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
        // following that way (as the manifest's gravity note has it).
        const RING_LED0_DEG = 112.5;
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
                at[v] = (((deg - RING_LED0_DEG) % 360) + 360) % 360 / 360;
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

        // The IMU's axes: x red, y green, z blue (out of the front).
        const axes = [
          [new THREE.Vector3(1, 0, 0), css("--err", "#fb7185")],
          [new THREE.Vector3(0, 1, 0), css("--ok", "#4ade80")],
          [new THREE.Vector3(0, 0, 1), css("--info", "#60a5fa")],
        ] as const;
        for (const [dir, color] of axes) {
          board.add(new THREE.ArrowHelper(dir, new THREE.Vector3(0, 0, 0), 0.05, new THREE.Color(color).getHex(), 0.008, 0.005));
        }

        const resize = () => {
          if (!host) return;
          const { clientWidth: w, clientHeight: h } = host;
          renderer.setSize(w, h, false);
          camera.aspect = w / Math.max(1, h);
          const vertical = (camera.fov * Math.PI) / 180;
          const horizontal = 2 * Math.atan(Math.tan(vertical / 2) * camera.aspect);
          const distance = FIT_RADIUS / Math.sin(Math.min(vertical, horizontal) / 2);
          camera.position.copy(VIEW_FROM).multiplyScalar(distance);
          camera.lookAt(0, 0, 0);
          camera.updateProjectionMatrix();
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
          renderer.render(scene, camera);
          frame = requestAnimationFrame(draw);
        };
        frame = requestAnimationFrame(draw);
        stop = () => {
          cancelAnimationFrame(frame);
          observer.disconnect();
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
  .model {
    width: 100%;
    height: 100%;
  }
  .model :global(canvas) {
    display: block;
    width: 100%;
    height: 100%;
  }
</style>
