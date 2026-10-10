<script lang="ts">
  // The Raze, as its case: the case model (static/models/raze-case.glb, built
  // by devices/tools/case_model.py from the CAD, in the IMU's frame) turned
  // by the IMU's orientation, with the lens added and the axes drawn. three.js
  // loads only when this shows. `onfail` says WebGL or the model isn't there,
  // so the caller can draw something simpler.
  import { onMount } from "svelte";
  import type { Quat } from "./imu.ts";

  let { orientation, onfail }: { orientation: () => Quat; onfail: () => void } = $props();

  let host = $state<HTMLDivElement>();

  onMount(() => {
    let stop = () => {};
    let gone = false;
    void (async () => {
      try {
        const THREE = await import("three");
        const { GLTFLoader } = await import("three/examples/jsm/loaders/GLTFLoader.js");
        if (gone || !host) return;
        const renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true });
        renderer.setPixelRatio(Math.min(2, window.devicePixelRatio || 1));
        host.appendChild(renderer.domElement);
        const scene = new THREE.Scene();
        const camera = new THREE.PerspectiveCamera(32, 1, 0.005, 1);
        // From the front, a little above and to the side.
        camera.position.set(0.075, 0.07, 0.11);
        camera.lookAt(0, 0, 0);
        scene.add(new THREE.HemisphereLight(0xdfe6f2, 0x20242c, 1.6));
        const key = new THREE.DirectionalLight(0xffffff, 2.2);
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
          TOP: new THREE.MeshStandardMaterial({ color: 0x2b2f36, roughness: 0.75, metalness: 0.05 }),
          BOTTOM: new THREE.MeshStandardMaterial({ color: 0x23262c, roughness: 0.8, metalness: 0.05 }),
          HEATSINK: new THREE.MeshStandardMaterial({ color: 0x9aa3ad, roughness: 0.35, metalness: 0.8 }),
          DIFFUSER: new THREE.MeshStandardMaterial({
            color: 0xf2f4f7,
            roughness: 0.4,
            transparent: true,
            opacity: 0.85,
            emissive: new THREE.Color(css("--accent", "#ff6b6b")),
            emissiveIntensity: 0.35,
          }),
          BUTTON: new THREE.MeshStandardMaterial({ color: 0x4a505a, roughness: 0.6 }),
        };
        const gltf = await new GLTFLoader().loadAsync("/models/raze-case.glb");
        gltf.scene.traverse((node) => {
          const mesh = node as InstanceType<typeof THREE.Mesh>;
          if (mesh.isMesh) mesh.material = MATERIALS[mesh.name] ?? MATERIALS[mesh.parent?.name ?? ""] ?? MATERIALS.TOP;
        });
        board.add(gltf.scene);
        // The lens, which the case model leaves out: dark glass in the ring's
        // centre on the front face.
        const front = new THREE.Box3().setFromObject(gltf.scene).max.z;
        const lens = new THREE.Mesh(
          new THREE.CylinderGeometry(0.0045, 0.0048, 0.004, 40),
          new THREE.MeshStandardMaterial({ color: 0x07090c, roughness: 0.08, metalness: 0.6 }),
        );
        lens.rotation.x = Math.PI / 2;
        lens.position.set(0, 0, front - 0.0015);
        board.add(lens);
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
          renderer.render(scene, camera);
          frame = requestAnimationFrame(draw);
        };
        frame = requestAnimationFrame(draw);
        stop = () => {
          cancelAnimationFrame(frame);
          observer.disconnect();
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
