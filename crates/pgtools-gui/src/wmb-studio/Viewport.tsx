import React, {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
  useState,
} from "react";
import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import { MaterialDesc, WmbSceneManifest } from "./contract";
import {
  collectDebugTextureSlots,
  computeSelectionBoundingBox,
  DebugChannelMode,
  DebugSlotPreview,
  findBatchById,
  findMaterialById,
  SceneSelectionItem,
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

    const [contextLost, setContextLost] = useState(false);
    const [debugPopoverOpen, setDebugPopoverOpen] = useState(false);
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

    const frameBoundingBox = useCallback(
      (bbox: number[] | null) => {
        const camera = cameraRef.current;
        const controls = controlsRef.current;
        if (!camera || !controls || !bbox || bbox.length < 6) return;

        const minX = bbox[0];
        const minY = bbox[1];
        const minZ = bbox[2];
        const maxX = bbox[3];
        const maxY = bbox[4];
        const maxZ = bbox[5];

        const cx = (minX + maxX) * 0.5;
        const cy = (minY + maxY) * 0.5;
        const cz = (minZ + maxZ) * 0.5;
        const dx = Math.max(0.1, maxX - minX);
        const dy = Math.max(0.1, maxY - minY);
        const dz = Math.max(0.1, maxZ - minZ);
        const radius = Math.max(0.25, Math.hypot(dx, dy, dz) * 0.5);

        // Fit model so height fills ~70% of viewport height
        const fovRad = (camera.fov * Math.PI) / 180;
        const targetHeight = Math.max(dy, radius * 1.35) / 0.7;
        const dist = Math.max(
          0.4,
          (targetHeight * 0.5) / Math.tan(fovRad * 0.5)
        );

        // Front-three-quarter elevation
        const dir = new THREE.Vector3(0.42, 0.26, 0.87).normalize();
        controls.target.set(cx, cy, cz);
        camera.position.set(
          cx + dir.x * dist,
          cy + dir.y * dist,
          cz + dir.z * dist
        );
        camera.near = Math.max(0.005, dist * 0.005);
        camera.far = Math.max(500, dist * 25);
        camera.updateProjectionMatrix();
        controls.update();
        requestRender();
      },
      [requestRender]
    );

    const frameAll = useCallback(() => {
      if (!manifest) return;
      frameBoundingBox(manifest.boundingBox);
    }, [frameBoundingBox, manifest]);

    const frameSelection = useCallback(() => {
      if (!manifest) return;
      const bbox = computeSelectionBoundingBox(manifest, selection);
      frameBoundingBox(bbox ?? manifest.boundingBox);
    }, [frameBoundingBox, manifest, selection]);

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
        frameBoundingBox(manifest.boundingBox);
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
      frameBoundingBox,
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
              title="Toggle wireframe rendering"
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
              title="Toggle world-space / normal-map normals view (N * 0.5 + 0.5)"
            >
              Normals
            </button>
            <button
              type="button"
              className={`wmb-vp-btn ${secondaryNormalsEnabled ? "active" : ""}`}
              onClick={() => setSecondaryNormalsEnabled((prev) => !prev)}
              title="Toggle NormalMap 2/3 & DetailNormalMap affecting surface normals (Off by default)"
            >
              NormalMap 2/3
            </button>
            <button
              type="button"
              className={`wmb-vp-btn ${skeletonOverlay ? "active" : ""}`}
              onClick={onToggleSkeletonOverlay}
              title="Toggle 3D skeleton overlay (joints & bones)"
            >
              Skeleton
            </button>
            <button
              type="button"
              className={`wmb-vp-btn ${gridVisible ? "active" : ""}`}
              onClick={onToggleGridVisible}
              title="Toggle ground grid & origin axes"
            >
              Grid
            </button>

            {manifest && manifest.lodLevels.length > 1 && (
              <label className="wmb-vp-lod-label" title="Filter WMB3 LOD level">
                <span>LOD:</span>
                <select
                  className="wmb-vp-lod-select"
                  value={activeLodLevel === null ? "all" : String(activeLodLevel)}
                  onChange={(e) => {
                    const val = e.target.value;
                    onChangeLodLevel(val === "all" ? null : parseInt(val, 10));
                  }}
                >
                  <option value="all">All LODs</option>
                  {manifest.lodLevels.map((lvl) => (
                    <option key={lvl} value={String(lvl)}>
                      LOD {lvl}
                    </option>
                  ))}
                </select>
              </label>
            )}

            <div className="wmb-vp-popover-anchor">
              <button
                type="button"
                className={`wmb-vp-btn ${debugSlot ? "active" : ""}`}
                onClick={() => setDebugPopoverOpen((prev) => !prev)}
                title="Preview individual texture slots and RGBA channels on the 3D model"
              >
                {debugSlot
                  ? `Tex: ${debugSlot.slotName} (${debugSlot.channel})`
                  : "Tex Debug ▾"}
              </button>

              {debugPopoverOpen && (
                <div className="wmb-vp-debug-popover">
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
                </div>
              )}
            </div>
          </div>

          <div className="wmb-viewport-toolbar-right">
            <button
              type="button"
              className="wmb-vp-btn"
              onClick={frameSelection}
              title="Frame active selection (F)"
            >
              Frame Sel (F)
            </button>
            <button
              type="button"
              className="wmb-vp-btn"
              onClick={frameAll}
              title="Frame entire model (Home)"
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
