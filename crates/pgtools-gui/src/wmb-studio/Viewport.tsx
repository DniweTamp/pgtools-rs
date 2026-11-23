import { StudioSelect } from "../components/StudioSelect";
import React, {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import { MaterialDesc, WmbSceneManifest } from "./contract";
import {
  collectBatchIdsForSelection,
  collectDebugTextureSlots,
  computeSelectionBoundingBox,
  DebugChannelMode,
  DebugSlotPreview,
  findBatchById,
  findMaterialById,
  isNullShaderName,
  SceneSelectionItem,
  sortLodLevelsByPriority,
  WmbGeometryCache,
} from "./store";
import {
  applyBatchHighlightUniforms,
  computeHighlightSets,
} from "./viewer/highlight";
import {
  buildSkeletonOverlay,
  createGridAndAxesOverlay,
  disposeObjectGroup,
} from "./viewer/overlays";
import { pickBatchAtNdc } from "./viewer/picking";
import {
  MaterialShaderStatus,
  SceneMeshBuilder,
} from "./viewer/sceneBuilder";
import { ViewerTextureCache } from "./viewer/textures";

export interface ViewportHandle {
  frameAll: () => void;
  frameSelection: () => void;
  updateMaterialLive: (mat: MaterialDesc) => void;
  getShaderStatuses: () => Map<string, MaterialShaderStatus>;
}

export interface ViewportProps {
  manifest: WmbSceneManifest | null;
  geomCache: WmbGeometryCache;
  selection: SceneSelectionItem[];
  onSelect: (items: SceneSelectionItem[], additive?: boolean) => void;
  theme: string;
  wireframe: boolean;
  onToggleWireframe: () => void;
  normalsView: boolean;
  onToggleNormalsView: () => void;
  skeletonOverlay: boolean;
  onToggleSkeletonOverlay: () => void;
  gridVisible: boolean;
  onToggleGridVisible: () => void;
  activeLodLevel: number | null;
  onChangeLodLevel: (level: number | null) => void;
  hiddenBatchIds: Set<string>;
  hiddenMeshGroupIds: Set<string>;
  onShaderStatusesChanged?: (statuses: Map<string, MaterialShaderStatus>) => void;
  debugSlot?: DebugSlotPreview | null;
  onChangeDebugSlot?: (slot: DebugSlotPreview | null) => void;
}

export const Viewport = React.memo(forwardRef<ViewportHandle, ViewportProps>(
  function Viewport(props, ref) {
    const {
      manifest,
      geomCache,
      selection,
      onSelect,
      theme,
      wireframe,
      onToggleWireframe,
      normalsView,
      onToggleNormalsView,
      skeletonOverlay,
      onToggleSkeletonOverlay,
      gridVisible,
      onToggleGridVisible,
      activeLodLevel,
      onChangeLodLevel,
      hiddenBatchIds,
      hiddenMeshGroupIds,
      onShaderStatusesChanged,
    } = props;

    const containerRef = useRef<HTMLDivElement | null>(null);
    const rendererRef = useRef<THREE.WebGLRenderer | null>(null);
    const sceneRef = useRef<THREE.Scene | null>(null);
    const cameraRef = useRef<THREE.PerspectiveCamera | null>(null);
    const controlsRef = useRef<OrbitControls | null>(null);
    const texCacheRef = useRef<ViewerTextureCache | null>(null);
    const meshBuilderRef = useRef<SceneMeshBuilder | null>(null);
    const gridGroupRef = useRef<THREE.Group | null>(null);
    const skeletonGroupRef = useRef<THREE.Group | null>(null);
    const rafIdRef = useRef<number | null>(null);
    const pointerDownPosRef = useRef<{ x: number; y: number } | null>(null);
    const lastFramedNodeKeyRef = useRef<string>("");

    const debugBtnRef = useRef<HTMLButtonElement | null>(null);
    const debugPopoverRef = useRef<HTMLDivElement | null>(null);

    const [contextLost, setContextLost] = useState(false);
    const [debugPopoverOpen, setDebugPopoverOpen] = useState(false);
    const [debugPopoverPos, setDebugPopoverPos] = useState<{
      left: number;
      top?: number;
      bottom?: number;
      width: number;
      maxHeight: number;
    }>({ left: 8, top: 48, width: 390, maxHeight: 360 });
    const [internalDebugSlot, setInternalDebugSlot] = useState<DebugSlotPreview | null>(null);
    const [secondaryNormalsEnabled, setSecondaryNormalsEnabled] = useState(false);

    const debugSlot =
      props.debugSlot !== undefined ? props.debugSlot : internalDebugSlot;
    const setDebugSlot = props.onChangeDebugSlot ?? setInternalDebugSlot;

    const effectiveDebugMode: DebugChannelMode = useMemo(() => {
      if (debugSlot) return "slot";
      if (normalsView) return "normals";
      return "off";
    }, [debugSlot, normalsView]);

    const requestRender = useCallback(() => {
      if (rafIdRef.current !== null) return;
      rafIdRef.current = window.requestAnimationFrame(() => {
        rafIdRef.current = null;
        const renderer = rendererRef.current;
        const scene = sceneRef.current;
        const camera = cameraRef.current;
        if (renderer && scene && camera) {
          renderer.render(scene, camera);
        }
      });
    }, []);

    const frameMeshesOrBoxes = useCallback(
      (meshes: THREE.Mesh[], fallbackBoxes: number[][]) => {
        const camera = cameraRef.current;
        const controls = controlsRef.current;
        if (!camera || !controls) return;

        let minX = Infinity;
        let minY = Infinity;
        let minZ = Infinity;
        let maxX = -Infinity;
        let maxY = -Infinity;
        let maxZ = -Infinity;
        let hasMeshVerts = false;

        for (const mesh of meshes) {
          const posAttr = mesh.geometry.getAttribute("position");
          if (!(posAttr instanceof THREE.BufferAttribute) || posAttr.count === 0) continue;
          const posArr = posAttr.array as ArrayLike<number>;
          const indexAttr = mesh.geometry.getIndex();
          if (indexAttr && indexAttr.count > 0) {
            const idxArr = indexAttr.array as ArrayLike<number>;
            const idxCount = indexAttr.count;
            const vCount = posAttr.count;
            for (let i = 0; i < idxCount; i++) {
              const vi = idxArr[i];
              if (vi >= vCount) continue;
              const base = vi * 3;
              const x = posArr[base];
              const y = posArr[base + 1];
              const z = posArr[base + 2];
              if (!Number.isFinite(x) || !Number.isFinite(y) || !Number.isFinite(z)) continue;
              if (x < minX) minX = x;
              if (y < minY) minY = y;
              if (z < minZ) minZ = z;
              if (x > maxX) maxX = x;
              if (y > maxY) maxY = y;
              if (z > maxZ) maxZ = z;
              hasMeshVerts = true;
            }
          } else {
            const vCount = posAttr.count;
            for (let vi = 0; vi < vCount; vi++) {
              const base = vi * 3;
              const x = posArr[base];
              const y = posArr[base + 1];
              const z = posArr[base + 2];
              if (!Number.isFinite(x) || !Number.isFinite(y) || !Number.isFinite(z)) continue;
              if (x < minX) minX = x;
              if (y < minY) minY = y;
              if (z < minZ) minZ = z;
              if (x > maxX) maxX = x;
              if (y > maxY) maxY = y;
              if (z > maxZ) maxZ = z;
              hasMeshVerts = true;
            }
          }
        }

        const validBoxes: number[][] = [];
        if (!hasMeshVerts) {
          for (const b of fallbackBoxes) {
            if (b.length < 6) continue;
            if (
              !Number.isFinite(b[0]) ||
              !Number.isFinite(b[1]) ||
              !Number.isFinite(b[2]) ||
              !Number.isFinite(b[3]) ||
              !Number.isFinite(b[4]) ||
              !Number.isFinite(b[5])
            ) {
              continue;
            }
            if (b[0] < minX) minX = b[0];
            if (b[1] < minY) minY = b[1];
            if (b[2] < minZ) minZ = b[2];
            if (b[3] > maxX) maxX = b[3];
            if (b[4] > maxY) maxY = b[4];
            if (b[5] > maxZ) maxZ = b[5];
            validBoxes.push(b);
          }
          if (validBoxes.length === 0) return;
        }

        if (
          !Number.isFinite(minX) ||
          !Number.isFinite(maxX) ||
          !Number.isFinite(minY) ||
          !Number.isFinite(maxY) ||
          !Number.isFinite(minZ) ||
          !Number.isFinite(maxZ)
        ) {
          return;
        }

        const cx = (minX + maxX) * 0.5;
        const cy = (minY + maxY) * 0.5;
        const cz = (minZ + maxZ) * 0.5;

        // Front-three-quarter elevation basis
        const dir = new THREE.Vector3(0.42, 0.26, 0.87).normalize();
        const right = new THREE.Vector3(0, 1, 0).cross(dir).normalize();
        const camUp = new THREE.Vector3().crossVectors(dir, right).normalize();
        const rx = right.x, ry = right.y, rz = right.z;
        const ux = camUp.x, uy = camUp.y, uz = camUp.z;
        const dx = dir.x, dy = dir.y, dz = dir.z;

        const container = containerRef.current;
        const aspect =
          container && container.clientHeight > 0
            ? Math.max(0.1, container.clientWidth / container.clientHeight)
            : Math.max(0.1, camera.aspect || 1);
        const tanHalfV = Math.tan(((camera.fov * Math.PI) / 180) * 0.5);
        const tanHalfH = tanHalfV * aspect;
        const fill = 0.82;
        const invV = 1 / (fill * tanHalfV);
        const invH = 1 / (fill * tanHalfH);

        let reqDist = 0;
        let minProjZ = Infinity;
        let maxProjZ = -Infinity;

        const considerPoint = (vx: number, vy: number, vz: number) => {
          const px = vx - cx;
          const py = vy - cy;
          const pz = vz - cz;
          const camX = px * rx + py * ry + pz * rz;
          const camY = px * ux + py * uy + pz * uz;
          const camZ = px * dx + py * dy + pz * dz;
          const dV = camZ + Math.abs(camY) * invV;
          const dH = camZ + Math.abs(camX) * invH;
          if (dV > reqDist) reqDist = dV;
          if (dH > reqDist) reqDist = dH;
          if (camZ < minProjZ) minProjZ = camZ;
          if (camZ > maxProjZ) maxProjZ = camZ;
        };

        if (hasMeshVerts) {
          for (const mesh of meshes) {
            const posAttr = mesh.geometry.getAttribute("position");
            if (!(posAttr instanceof THREE.BufferAttribute) || posAttr.count === 0) continue;
            const posArr = posAttr.array as ArrayLike<number>;
            const indexAttr = mesh.geometry.getIndex();
            if (indexAttr && indexAttr.count > 0) {
              const idxArr = indexAttr.array as ArrayLike<number>;
              const idxCount = indexAttr.count;
              const vCount = posAttr.count;
              for (let i = 0; i < idxCount; i++) {
                const vi = idxArr[i];
                if (vi >= vCount) continue;
                const base = vi * 3;
                const x = posArr[base];
                const y = posArr[base + 1];
                const z = posArr[base + 2];
                if (!Number.isFinite(x) || !Number.isFinite(y) || !Number.isFinite(z)) continue;
                considerPoint(x, y, z);
              }
            } else {
              const vCount = posAttr.count;
              for (let vi = 0; vi < vCount; vi++) {
                const base = vi * 3;
                const x = posArr[base];
                const y = posArr[base + 1];
                const z = posArr[base + 2];
                if (!Number.isFinite(x) || !Number.isFinite(y) || !Number.isFinite(z)) continue;
                considerPoint(x, y, z);
              }
            }
          }
        } else {
          for (const b of validBoxes) {
            for (const x of [b[0], b[3]]) {
              for (const y of [b[1], b[4]]) {
                for (const z of [b[2], b[5]]) {
                  considerPoint(x, y, z);
                }
              }
            }
          }
        }

        if (!Number.isFinite(minProjZ)) minProjZ = -0.25;
        if (!Number.isFinite(maxProjZ)) maxProjZ = 0.25;

        const dist = Math.max(0.25, reqDist, maxProjZ + 0.05);
        controls.target.set(cx, cy, cz);
        camera.position.set(
          cx + dx * dist,
          cy + dy * dist,
          cz + dz * dist
        );
        const frontClearance = Math.max(0.002, dist - maxProjZ);
        camera.near = Math.max(0.002, Math.min(dist * 0.01, frontClearance * 0.25));
        camera.far = Math.max(500, (dist - minProjZ) * 10, dist * 25);
        camera.updateProjectionMatrix();
        controls.update();
        requestRender();
      },
      [requestRender]
    );

    const frameAll = useCallback(() => {
      if (!manifest) return;
      const visibleMeshes: THREE.Mesh[] = [];
      const meshBuilder = meshBuilderRef.current;
      if (meshBuilder) {
        meshBuilder.batchMeshes.forEach((mesh) => {
          if (mesh.visible) visibleMeshes.push(mesh);
        });
      }
      if (visibleMeshes.length > 0) {
        frameMeshesOrBoxes(visibleMeshes, []);
        return;
      }
      const matShaderById = new Map<string, string | null>();
      for (const m of manifest.materials) {
        matShaderById.set(m.id, m.shaderName);
      }
      const fallbackBoxes = manifest.batches
        .filter(
          (b) =>
            !b.isShadowProxy &&
            !isNullShaderName(matShaderById.get(b.materialId)) &&
            b.bbox.length === 6 &&
            b.indexCount > 0
        )
        .map((b) => b.bbox);
      if (fallbackBoxes.length > 0) {
        frameMeshesOrBoxes([], fallbackBoxes);
      } else if (manifest.boundingBox.length === 6) {
        frameMeshesOrBoxes([], [manifest.boundingBox]);
      }
    }, [frameMeshesOrBoxes, manifest]);

    const frameSelection = useCallback(() => {
      if (!manifest) return;
      if (
        selection.length === 0 ||
        selection.some((s) => s.kind === "root")
      ) {
        frameAll();
        return;
      }
      const hasBoneSel = selection.some((s) => s.kind === "bone");
      if (!hasBoneSel && meshBuilderRef.current) {
        const selBatchIds = collectBatchIdsForSelection(manifest, selection, false);
        const selMeshes: THREE.Mesh[] = [];
        selBatchIds.forEach((bid) => {
          const m = meshBuilderRef.current?.batchMeshes.get(bid);
          if (m && m.visible) selMeshes.push(m);
        });
        if (selMeshes.length > 0) {
          frameMeshesOrBoxes(selMeshes, []);
          return;
        }
      }
      const bbox = computeSelectionBoundingBox(manifest, selection);
      if (bbox) {
        frameMeshesOrBoxes([], [bbox]);
      } else {
        frameAll();
      }
    }, [frameAll, frameMeshesOrBoxes, manifest, selection]);

    useImperativeHandle(
      ref,
      () => ({
        frameAll,
        frameSelection,
        updateMaterialLive: (mat: MaterialDesc) => {
          meshBuilderRef.current?.updateMaterialInPlace(mat);
        },
        getShaderStatuses: () =>
          meshBuilderRef.current?.materialShaderStatus ?? new Map(),
      }),
      [frameAll, frameSelection]
    );

      useEffect(() => {
      const container = containerRef.current;
      if (!container) return;

      const width = Math.max(1, container.clientWidth);
      const height = Math.max(1, container.clientHeight);

      const renderer = new THREE.WebGLRenderer({
        antialias: true,
        powerPreference: "high-performance",
      });
      renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
      renderer.setSize(width, height, false);
      renderer.outputColorSpace = THREE.SRGBColorSpace;
      renderer.domElement.className = "wmb-viewport-canvas";
      container.appendChild(renderer.domElement);

      const scene = new THREE.Scene();
      scene.background = new THREE.Color(
        theme === "light" ? 0xf1f5f9 : 0x0b101b
      );

      const camera = new THREE.PerspectiveCamera(45, width / height, 0.01, 500);
      camera.position.set(1.2, 1.4, 2.6);

      const controls = new OrbitControls(camera, renderer.domElement);
      controls.enableDamping = false;
      controls.screenSpacePanning = true;
      controls.target.set(0, 0.9, 0);
      controls.mouseButtons = {
        LEFT: THREE.MOUSE.ROTATE,
        MIDDLE: THREE.MOUSE.PAN,
        RIGHT: THREE.MOUSE.PAN,
      };
      controls.update();

      const onControlsChange = () => requestRender();
      controls.addEventListener("change", onControlsChange);

      const texCache = new ViewerTextureCache();
      const meshBuilder = new SceneMeshBuilder(texCache, requestRender);
      scene.add(meshBuilder.rootGroup);

      const gridGroup = createGridAndAxesOverlay(theme);
      gridGroup.visible = gridVisible;
      scene.add(gridGroup);

      rendererRef.current = renderer;
      sceneRef.current = scene;
      cameraRef.current = camera;
      controlsRef.current = controls;
      texCacheRef.current = texCache;
      meshBuilderRef.current = meshBuilder;
      gridGroupRef.current = gridGroup;

      const handleContextLost = (e: Event) => {
        e.preventDefault();
        setContextLost(true);
      };
      const handleContextRestored = () => {
        setContextLost(false);
        requestRender();
      };
      renderer.domElement.addEventListener(
        "webglcontextlost",
        handleContextLost
      );
      renderer.domElement.addEventListener(
        "webglcontextrestored",
        handleContextRestored
      );

      const resizeObserver = new ResizeObserver((entries) => {
        for (const entry of entries) {
          const cw = Math.max(1, Math.floor(entry.contentRect.width));
          const ch = Math.max(1, Math.floor(entry.contentRect.height));
          if (cameraRef.current && rendererRef.current) {
            cameraRef.current.aspect = cw / ch;
            cameraRef.current.updateProjectionMatrix();
            rendererRef.current.setSize(cw, ch, false);
            requestRender();
          }
        }
      });
      resizeObserver.observe(container);

      requestRender();

      return () => {
        resizeObserver.disconnect();
        controls.removeEventListener("change", onControlsChange);
        controls.dispose();
        renderer.domElement.removeEventListener(
          "webglcontextlost",
          handleContextLost
        );
        renderer.domElement.removeEventListener(
          "webglcontextrestored",
          handleContextRestored
        );
        if (rafIdRef.current !== null) {
          window.cancelAnimationFrame(rafIdRef.current);
          rafIdRef.current = null;
        }
        if (gridGroupRef.current) {
          disposeObjectGroup(gridGroupRef.current);
          gridGroupRef.current = null;
        }
        if (skeletonGroupRef.current) {
          disposeObjectGroup(skeletonGroupRef.current);
          skeletonGroupRef.current = null;
        }
        meshBuilder.dispose();
        texCache.dispose();
        renderer.dispose();
        if (renderer.domElement.parentNode === container) {
          container.removeChild(renderer.domElement);
        }
      };
    }, []); // Mount once

      useEffect(() => {
      const scene = sceneRef.current;
      if (!scene) return;
      scene.background = new THREE.Color(
        theme === "light" ? 0xf1f5f9 : 0x0b101b
      );
      if (gridGroupRef.current) {
        scene.remove(gridGroupRef.current);
        disposeObjectGroup(gridGroupRef.current);
      }
      const nextGrid = createGridAndAxesOverlay(theme);
      nextGrid.visible = gridVisible;
      scene.add(nextGrid);
      gridGroupRef.current = nextGrid;
      requestRender();
    }, [theme, gridVisible, requestRender]);

      useEffect(() => {
      const meshBuilder = meshBuilderRef.current;
      const renderer = rendererRef.current;
      if (!meshBuilder || !renderer || !manifest) return;

      const gl = renderer.getContext();
      meshBuilder.syncScene(manifest, geomCache, gl, {
        wireframe,
        debugMode: effectiveDebugMode,
        debugSlot,
        activeLodLevel,
        hiddenBatchIds,
        hiddenMeshGroupIds,
        secondaryNormalsEnabled,
      });

      const highlights = computeHighlightSets(manifest, selection);
      applyBatchHighlightUniforms(meshBuilder.batchMeshes, highlights.batchIds);

      if (onShaderStatusesChanged) {
        onShaderStatusesChanged(new Map(meshBuilder.materialShaderStatus));
      }

      const nodeKey = `${manifest.nodeId}:${manifest.fileName}`;
      if (lastFramedNodeKeyRef.current !== nodeKey) {
        lastFramedNodeKeyRef.current = nodeKey;
        frameAll();
      } else {
        requestRender();
      }
    }, [
      manifest,
      geomCache,
      wireframe,
      effectiveDebugMode,
      debugSlot,
      secondaryNormalsEnabled,
      activeLodLevel,
      hiddenBatchIds,
      hiddenMeshGroupIds,
      selection,
      frameAll,
      requestRender,
      onShaderStatusesChanged,
      contextLost,
    ]);

      useEffect(() => {
      const scene = sceneRef.current;
      const meshBuilder = meshBuilderRef.current;
      if (!scene || !meshBuilder) return;

      const highlights = computeHighlightSets(manifest, selection);
      applyBatchHighlightUniforms(meshBuilder.batchMeshes, highlights.batchIds);

      if (skeletonGroupRef.current) {
        scene.remove(skeletonGroupRef.current);
        disposeObjectGroup(skeletonGroupRef.current);
        skeletonGroupRef.current = null;
      }

      if (skeletonOverlay && manifest && manifest.bones.length > 0) {
        const skel = buildSkeletonOverlay(manifest.bones, highlights.boneIds);
        scene.add(skel);
        skeletonGroupRef.current = skel;
      }

      requestRender();
    }, [manifest, selection, skeletonOverlay, requestRender]);

      const handleKeyDown = useCallback(
      (e: React.KeyboardEvent<HTMLDivElement>) => {
        if (
          e.target instanceof HTMLInputElement ||
          e.target instanceof HTMLTextAreaElement ||
          e.target instanceof HTMLSelectElement
        ) {
          return;
        }
        if (e.key === "f" || e.key === "F") {
          e.preventDefault();
          frameSelection();
        } else if (e.key === "Home") {
          e.preventDefault();
          frameAll();
        }
      },
      [frameAll, frameSelection]
    );

    const handlePointerDown = useCallback(
      (e: React.PointerEvent<HTMLDivElement>) => {
        if (e.button === 0) {
          pointerDownPosRef.current = { x: e.clientX, y: e.clientY };
        }
      },
      []
    );

    const handlePointerUp = useCallback(
      (e: React.PointerEvent<HTMLDivElement>) => {
        if (e.button !== 0 || !pointerDownPosRef.current) return;
        const dx = e.clientX - pointerDownPosRef.current.x;
        const dy = e.clientY - pointerDownPosRef.current.y;
        pointerDownPosRef.current = null;

        // Ignore if user dragged to orbit/pan
        if (Math.hypot(dx, dy) > 4 || e.shiftKey) return;

        const container = containerRef.current;
        const camera = cameraRef.current;
        const meshBuilder = meshBuilderRef.current;
        if (!container || !camera || !meshBuilder || !manifest) return;

        const rect = container.getBoundingClientRect();
        if (rect.width <= 0 || rect.height <= 0) return;

        const ndcX = ((e.clientX - rect.left) / rect.width) * 2 - 1;
        const ndcY = -((e.clientY - rect.top) / rect.height) * 2 + 1;

        const hit = pickBatchAtNdc(
          ndcX,
          ndcY,
          camera,
          meshBuilder.batchMeshes
        );
        if (hit) {
          const additive = e.ctrlKey || e.metaKey;
          if (e.altKey) {
            onSelect([{ kind: "material", id: hit.materialId }], additive);
          } else {
            onSelect([{ kind: "batch", id: hit.batchId }], additive);
          }
        }
      },
      [manifest, onSelect]
    );

    // Texture slots for the Texture debug preview popover:
    // If a material or batch is selected, prioritize that material's texture slots; otherwise collect across manifest.
    const debugSlotRows = useMemo(() => {
      if (!manifest) return [];
      const firstSel = selection[0];
      let targetMat: MaterialDesc | undefined;
      if (firstSel?.kind === "material") {
        targetMat = findMaterialById(manifest, firstSel.id);
      } else if (firstSel?.kind === "batch") {
        const b = findBatchById(manifest, firstSel.id);
        if (b) targetMat = findMaterialById(manifest, b.materialId);
      }
      return collectDebugTextureSlots(manifest, targetMat);
    }, [manifest, selection]);

    const sortedLodLevels = useMemo(
      () => sortLodLevelsByPriority(manifest?.lodLevels ?? []),
      [manifest?.lodLevels]
    );

    const updateDebugPopoverPosition = useCallback(() => {
      const btn = debugBtnRef.current;
      if (!btn) return;
      const rect = btn.getBoundingClientRect();
      const vw = window.innerWidth;
      const vh = window.innerHeight;
      const margin = 8;
      const width = Math.min(390, Math.max(220, vw - margin * 2));
      const left = Math.max(margin, Math.min(rect.left, vw - width - margin));
      const desiredMaxH = 360;
      const spaceBelow = vh - rect.bottom - margin - 4;
      const spaceAbove = rect.top - margin - 4;
      const openUp = spaceBelow < 180 && spaceAbove > spaceBelow;
      const maxHeight = Math.max(
        120,
        Math.min(desiredMaxH, openUp ? spaceAbove : spaceBelow)
      );
      if (openUp) {
        const bottom = Math.max(margin, vh - rect.top + 4);
        setDebugPopoverPos({ left, bottom, width, maxHeight });
      } else {
        const top = Math.max(margin, Math.min(vh - margin - 120, rect.bottom + 4));
        setDebugPopoverPos({ left, top, width, maxHeight });
      }
    }, []);

    useLayoutEffect(() => {
      if (!debugPopoverOpen) return;
      updateDebugPopoverPosition();
    }, [debugPopoverOpen, debugSlotRows.length, updateDebugPopoverPosition]);

    useEffect(() => {
      if (!debugPopoverOpen) return;
      const onPointerDown = (e: MouseEvent) => {
        const target = e.target as Node | null;
        if (
          debugBtnRef.current?.contains(target) ||
          debugPopoverRef.current?.contains(target)
        ) {
          return;
        }
        setDebugPopoverOpen(false);
      };
      const onKeyDown = (e: KeyboardEvent) => {
        if (e.key === "Escape") {
          e.preventDefault();
          e.stopPropagation();
          setDebugPopoverOpen(false);
          debugBtnRef.current?.focus();
        }
      };
      const onViewportChange = () => updateDebugPopoverPosition();
      document.addEventListener("mousedown", onPointerDown, true);
      document.addEventListener("keydown", onKeyDown, true);
      window.addEventListener("resize", onViewportChange);
      window.addEventListener("scroll", onViewportChange, true);
      return () => {
        document.removeEventListener("mousedown", onPointerDown, true);
        document.removeEventListener("keydown", onKeyDown, true);
        window.removeEventListener("resize", onViewportChange);
        window.removeEventListener("scroll", onViewportChange, true);
      };
    }, [debugPopoverOpen, updateDebugPopoverPosition]);

    const channels: Array<"RGB" | "R" | "G" | "B" | "A"> = [
      "RGB",
      "R",
      "G",
      "B",
      "A",
    ];

    return (
      <div
        className="wmb-viewport-shell"
        tabIndex={0}
        onKeyDown={handleKeyDown}
      >
        {/* Top compact viewport toolbar */}
        <div className="wmb-viewport-toolbar">
          <div className="wmb-viewport-toolbar-group">
            <button
              type="button"
              className={`wmb-vp-btn ${wireframe ? "active" : ""}`}
              onClick={onToggleWireframe}
              data-tooltip="Toggle wireframe rendering"
            >
              Wireframe
            </button>
            <button
              type="button"
              className={`wmb-vp-btn ${
                normalsView && !debugSlot ? "active" : ""
              }`}
              onClick={() => {
                if (debugSlot) setDebugSlot(null);
                onToggleNormalsView();
              }}
              data-tooltip="Toggle world-space / normal-map normals view (N * 0.5 + 0.5)"
            >
              Normals
            </button>
            <button
              type="button"
              className={`wmb-vp-btn ${secondaryNormalsEnabled ? "active" : ""}`}
              onClick={() => setSecondaryNormalsEnabled((prev) => !prev)}
              data-tooltip="Toggle NormalMap 2/3 & DetailNormalMap affecting surface normals (Off by default)"
            >
              NormalMap 2/3
            </button>
            <button
              type="button"
              className={`wmb-vp-btn ${skeletonOverlay ? "active" : ""}`}
              onClick={onToggleSkeletonOverlay}
              data-tooltip="Toggle 3D skeleton overlay (joints & bones)"
            >
              Skeleton
            </button>
            <button
              type="button"
              className={`wmb-vp-btn ${gridVisible ? "active" : ""}`}
              onClick={onToggleGridVisible}
              data-tooltip="Toggle ground grid & origin axes"
            >
              Grid
            </button>

            {manifest && sortedLodLevels.length > 1 && (
              <div className="wmb-vp-lod-label" data-tooltip="Filter WMB3 LOD level">
                <span>LOD:</span>
                <StudioSelect
                  className="wmb-vp-lod-select"
                  value={activeLodLevel === null ? "all" : String(activeLodLevel)}
                  options={[
                    { value: "all", label: "All LODs" },
                    ...sortedLodLevels.map((lvl) => ({
                      value: String(lvl),
                      label: `LOD ${lvl}`,
                    })),
                  ]}
                  onChange={(val) => {
                    onChangeLodLevel(val === "all" ? null : parseInt(val, 10));
                  }}
                />
              </div>
            )}

            <div className="wmb-vp-popover-anchor">
              <button
                ref={debugBtnRef}
                type="button"
                className={`wmb-vp-btn ${debugSlot ? "active" : ""}`}
                onClick={() => setDebugPopoverOpen((prev) => !prev)}
                data-tooltip="Preview individual texture slots and RGBA channels on the 3D model"
              >
                {debugSlot
                  ? `Tex: ${debugSlot.slotName} (${debugSlot.channel})`
                  : "Tex Debug ▾"}
              </button>

              {debugPopoverOpen &&
                createPortal(
                  <div
                    ref={debugPopoverRef}
                    className="wmb-vp-debug-popover"
                    style={{
                      left: `${debugPopoverPos.left}px`,
                      top:
                        debugPopoverPos.top !== undefined
                          ? `${debugPopoverPos.top}px`
                          : undefined,
                      bottom:
                        debugPopoverPos.bottom !== undefined
                          ? `${debugPopoverPos.bottom}px`
                          : undefined,
                      width: `${debugPopoverPos.width}px`,
                      maxHeight: `${debugPopoverPos.maxHeight}px`,
                    }}
                  >
                    <div className="wmb-vp-debug-header">
                      <span>Texture Channel Preview</span>
                      <button
                        type="button"
                        className={`wmb-vp-channel-pill ${
                          !debugSlot ? "active" : ""
                        }`}
                        onClick={() => setDebugSlot(null)}
                      >
                        Off (Shaded)
                      </button>
                    </div>
                    {debugSlotRows.length === 0 ? (
                      <div className="wmb-vp-debug-empty">
                        No texture slots on this model.
                      </div>
                    ) : (
                      <div className="wmb-vp-debug-grid">
                        {debugSlotRows.map((row) => (
                          <div key={row.slotIndex} className="wmb-vp-debug-row">
                            <div className="wmb-vp-debug-slot-info">
                              {row.thumbnailDataUrl ? (
                                <img
                                  src={row.thumbnailDataUrl}
                                  alt={row.name}
                                  className="wmb-vp-debug-thumb"
                                />
                              ) : (
                                <div className="wmb-vp-debug-thumb-placeholder">
                                  #{row.slotIndex}
                                </div>
                              )}
                              <div className="wmb-vp-debug-meta">
                                <span className="wmb-vp-debug-name">
                                  {row.name}
                                </span>
                                <span
                                  className={`wmb-role-badge role-${row.role}`}
                                >
                                  {row.role}
                                </span>
                              </div>
                            </div>
                            <div className="wmb-vp-channel-pills">
                              {channels.map((ch) => {
                                const isActive =
                                  debugSlot?.slotIndex === row.slotIndex &&
                                  debugSlot?.channel === ch;
                                return (
                                  <button
                                    key={ch}
                                    type="button"
                                    className={`wmb-vp-channel-pill ${
                                      isActive ? "active" : ""
                                    }`}
                                    onClick={() => {
                                      if (isActive) {
                                        setDebugSlot(null);
                                      } else {
                                        setDebugSlot({
                                          slotIndex: row.slotIndex,
                                          slotName: row.name,
                                          role: row.role,
                                          channel: ch,
                                        });
                                      }
                                    }}
                                  >
                                    {ch}
                                  </button>
                                );
                              })}
                            </div>
                          </div>
                        ))}
                      </div>
                    )}
                  </div>,
                  document.body
                )}
            </div>
          </div>

          <div className="wmb-viewport-toolbar-right">
            <button
              type="button"
              className="wmb-vp-btn"
              onClick={frameSelection}
              data-tooltip="Frame active selection (F)"
            >
              Frame Sel (F)
            </button>
            <button
              type="button"
              className="wmb-vp-btn"
              onClick={frameAll}
              data-tooltip="Frame entire model (Home)"
            >
              Reset View
            </button>
          </div>
        </div>

        {/* WebGL Canvas Container */}
        <div
          ref={containerRef}
          className="wmb-viewport-canvas-wrap"
          onPointerDown={handlePointerDown}
          onPointerUp={handlePointerUp}
        >
          {contextLost && (
            <div className="wmb-viewport-context-banner">
              WebGL context lost — waiting for GPU recovery…
            </div>
          )}
          <div className="wmb-viewport-hud-hint">
            LMB: Orbit · MMB/Shift+LMB: Pan · Wheel: Zoom · Click: Pick Batch ·
            Alt+Click: Pick Material · F: Frame
          </div>
        </div>
      </div>
    );
  }
));
