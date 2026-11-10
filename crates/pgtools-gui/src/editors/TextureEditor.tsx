import React, { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import * as THREE from "three";
import {
  ContextMenuState,
  CubemapPreviewPayload,
  MenuItemSpec,
  TexModalState,
  TextureNodeView,
  UvdContextView,
} from "../types";
import { UvdActions, UvdInspectorSection } from "./UvdEditor";
import {
  IconCheck,
  IconChevronDown,
  IconCube,
  IconImage,
  IconRefresh,
} from "../components/Icons";

export const ALL_TEXTURE_ENCODE_FORMATS: {
  group: string;
  options: { value: string; label: string }[];
}[] = [
  {
    group: "Automatic",
    options: [{ value: "Auto", label: "Auto (Match Existing Texture Format)" }],
  },
  {
    group: "BCn Block Compression (DX9 / DX10)",
    options: [
      { value: "BC1_UNORM", label: "BC1_UNORM (DXT1 — 4 bpp)" },
      { value: "BC1_SRGB", label: "BC1_SRGB (DXT1 sRGB)" },
      { value: "BC2_UNORM", label: "BC2_UNORM (DXT3 — Explicit Alpha)" },
      { value: "BC2_SRGB", label: "BC2_SRGB (DXT3 sRGB)" },
      { value: "BC3_UNORM", label: "BC3_UNORM (DXT5 — Interpolated Alpha)" },
      { value: "BC3_SRGB", label: "BC3_SRGB (DXT5 sRGB)" },
      { value: "BC4_UNORM", label: "BC4_UNORM (ATI1 — Single Channel)" },
      { value: "BC5_UNORM", label: "BC5_UNORM (ATI2 — Dual Channel Normal)" },
      { value: "BC6H_UF16", label: "BC6H_UF16 (DX10 HDR Unsigned Float)" },
      { value: "BC6H_SF16", label: "BC6H_SF16 (DX10 HDR Signed Float)" },
      { value: "BC7_UNORM", label: "BC7_UNORM (DX10 High-Quality RGBA)" },
      { value: "BC7_SRGB", label: "BC7_SRGB (DX10 High-Quality sRGB)" },
    ],
  },
  {
    group: "ASTC LDR (Switch XT1 / BNTX / DX10 — All 14 Footprints)",
    options: [
      "4x4",
      "5x4",
      "5x5",
      "6x5",
      "6x6",
      "8x5",
      "8x6",
      "8x8",
      "10x5",
      "10x6",
      "10x8",
      "10x10",
      "12x10",
      "12x12",
    ].flatMap((fp) => [
      { value: `ASTC_${fp}_UNORM`, label: `ASTC_${fp}_UNORM (Linear)` },
      { value: `ASTC_${fp}_SRGB`, label: `ASTC_${fp}_SRGB (sRGB)` },
    ]),
  },
  {
    group: "Uncompressed / Float Formats",
    options: [
      { value: "RGBA8_UNORM", label: "RGBA8_UNORM (32-bit)" },
      { value: "RGBA8_SRGB", label: "RGBA8_SRGB (32-bit sRGB)" },
      { value: "BGRA8_UNORM", label: "BGRA8_UNORM (32-bit)" },
      { value: "BGRA8_SRGB", label: "BGRA8_SRGB (32-bit sRGB)" },
      { value: "RG8_UNORM", label: "R8G8_UNORM (16-bit)" },
      { value: "R8_UNORM", label: "R8_UNORM / AF_L8_UNORM (8-bit)" },
      { value: "RGBA16_FLOAT", label: "RGBA16_FLOAT (64-bit Half Float)" },
    ],
  },
];

export const ENCODE_QUALITY_OPTIONS = [
  { value: "Fast", label: "Fast" },
  { value: "Normal", label: "Normal (Default)" },
  { value: "Production", label: "Production" },
  { value: "Highest", label: "Highest" },
];

interface CubemapSkyViewportProps {
  nodeId: string;
  mipIndex: number;
  opaquePreview: boolean;
}

const CubemapSkyViewport: React.FC<CubemapSkyViewportProps> = ({
  nodeId,
  mipIndex,
  opaquePreview,
}) => {
  const mountRef = useRef<HTMLDivElement | null>(null);
  const [gammaDraft, setGammaDraft] = useState("1.00");
  const [exposureDraft, setExposureDraft] = useState("1.00");
  const [gamma, setGamma] = useState(1.0);
  const [exposure, setExposure] = useState(1.0);
  const [autoExposure, setAutoExposure] = useState(true);
  const [autoScale, setAutoScale] = useState(1.0);
  const [loadingErr, setLoadingErr] = useState<string | null>(null);

  const uniformsRef = useRef<{
    uCube: { value: THREE.CubeTexture | null };
    uExposure: { value: number };
    uGamma: { value: number };
  }>({
    uCube: { value: null },
    uExposure: { value: 1.0 },
    uGamma: { value: 1.0 },
  });

  useEffect(() => {
    const effExp = exposure * (autoExposure ? autoScale : 1.0);
    uniformsRef.current.uExposure.value = effExp;
    uniformsRef.current.uGamma.value = Math.max(0.1, gamma);
  }, [exposure, gamma, autoExposure, autoScale]);

  useEffect(() => {
    let cancelled = false;
    setLoadingErr(null);

    (async () => {
      try {
        const res = await invoke<CubemapPreviewPayload>("get_cubemap_faces_preview", {
          nodeId,
          mipIndex,
          opaque: opaquePreview,
        });
        if (cancelled) return;
        setAutoScale(res.auto_exposure_scale || 1.0);

        const loader = new THREE.ImageLoader();
        const images = await Promise.all(
          res.face_data_urls.map(
            (url) =>
              new Promise<HTMLImageElement>((resolve, reject) => {
                loader.load(url, resolve, undefined, reject);
              })
          )
        );
        if (cancelled) return;
        const cubeTex = new THREE.CubeTexture(images);
        cubeTex.needsUpdate = true;
        if (uniformsRef.current.uCube.value) {
          uniformsRef.current.uCube.value.dispose();
        }
        uniformsRef.current.uCube.value = cubeTex;
      } catch (err) {
        if (!cancelled) {
          setLoadingErr(String(err));
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [nodeId, mipIndex, opaquePreview]);

  useEffect(() => {
    const host = mountRef.current;
    if (!host) return;

    const scene = new THREE.Scene();
    const camera = new THREE.PerspectiveCamera(
      60,
      Math.max(1, host.clientWidth) / Math.max(1, host.clientHeight),
      0.1,
      100
    );
    camera.position.set(0, 0, 0.01);

    const renderer = new THREE.WebGLRenderer({ antialias: true });
    renderer.setSize(host.clientWidth, host.clientHeight);
    renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    host.innerHTML = "";
    host.appendChild(renderer.domElement);

    const skyMat = new THREE.ShaderMaterial({
      uniforms: uniformsRef.current,
      vertexShader: `
        varying vec3 vDir;
        void main() {
          vDir = position;
          gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
        }
      `,
      fragmentShader: `
        uniform samplerCube uCube;
        uniform float uExposure;
        uniform float uGamma;
        varying vec3 vDir;
        void main() {
          vec4 texel = textureCube(uCube, normalize(vDir));
          vec3 col = texel.rgb * uExposure;
          col = pow(max(col, vec3(0.0)), vec3(1.0 / max(uGamma, 0.1)));
          gl_FragColor = vec4(col, 1.0);
        }
      `,
      side: THREE.BackSide,
      depthWrite: false,
    });

    const skyGeo = new THREE.BoxGeometry(10, 10, 10);
    const skyMesh = new THREE.Mesh(skyGeo, skyMat);
    scene.add(skyMesh);

    let yaw = 0;
    let pitch = 0;
    let fov = 60;
    let dragging = false;
    let lastX = 0;
    let lastY = 0;
    let animId = 0;

    const updateCamera = () => {
      camera.fov = fov;
      camera.updateProjectionMatrix();
      const phi = THREE.MathUtils.degToRad(90 - pitch);
      const theta = THREE.MathUtils.degToRad(yaw);
      const target = new THREE.Vector3(
        Math.sin(phi) * Math.cos(theta),
        Math.cos(phi),
        Math.sin(phi) * Math.sin(theta)
      );
      camera.lookAt(target);
    };
    updateCamera();

    const onMouseDown = (e: MouseEvent) => {
      if (e.button === 0) {
        dragging = true;
        lastX = e.clientX;
        lastY = e.clientY;
      }
    };
    const onMouseMove = (e: MouseEvent) => {
      if (!dragging) return;
      const dx = e.clientX - lastX;
      const dy = e.clientY - lastY;
      lastX = e.clientX;
      lastY = e.clientY;
      yaw -= dx * 0.25;
      pitch = Math.max(-89, Math.min(89, pitch + dy * 0.25));
      updateCamera();
    };
    const onMouseUp = () => {
      dragging = false;
    };
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      fov = Math.max(20, Math.min(100, fov + (e.deltaY > 0 ? 4 : -4)));
      updateCamera();
    };
    const onDblClick = () => {
      yaw = 0;
      pitch = 0;
      fov = 60;
      updateCamera();
    };

    const dom = renderer.domElement;
    dom.addEventListener("mousedown", onMouseDown);
    window.addEventListener("mousemove", onMouseMove);
    window.addEventListener("mouseup", onMouseUp);
    dom.addEventListener("wheel", onWheel, { passive: false });
    dom.addEventListener("dblclick", onDblClick);

    const ro = new ResizeObserver(() => {
      if (!host) return;
      const w = Math.max(1, host.clientWidth);
      const h = Math.max(1, host.clientHeight);
      camera.aspect = w / h;
      camera.updateProjectionMatrix();
      renderer.setSize(w, h);
    });
    ro.observe(host);

    const renderLoop = () => {
      animId = requestAnimationFrame(renderLoop);
      renderer.render(scene, camera);
    };
    renderLoop();

    return () => {
      cancelAnimationFrame(animId);
      ro.disconnect();
      dom.removeEventListener("mousedown", onMouseDown);
      window.removeEventListener("mousemove", onMouseMove);
      window.removeEventListener("mouseup", onMouseUp);
      dom.removeEventListener("wheel", onWheel);
      dom.removeEventListener("dblclick", onDblClick);
      if (uniformsRef.current.uCube.value) {
        uniformsRef.current.uCube.value.dispose();
        uniformsRef.current.uCube.value = null;
      }
      skyGeo.dispose();
      skyMat.dispose();
      renderer.dispose();
    };
  }, []);

  const commitGammaExposure = () => {
    const g = Math.max(0.1, Math.min(8.0, parseFloat(gammaDraft) || 1.0));
    const ex = Math.max(0.05, Math.min(20.0, parseFloat(exposureDraft) || 1.0));
    setGamma(g);
    setExposure(ex);
    setGammaDraft(g.toFixed(2));
    setExposureDraft(ex.toFixed(2));
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", flex: 1, minHeight: 0 }}>
      <div
        ref={mountRef}
        style={{ flex: 1, minHeight: 0, position: "relative", cursor: "grab" }}
        title="Drag to orbit cubemap sky, wheel to zoom, double-click to reset camera"
      />
      {loadingErr && (
        <div
          style={{
            padding: "6px 12px",
            color: "var(--brush-warn)",
            fontSize: 11.5,
          }}
        >
          Cubemap preview warning: {loadingErr}
        </div>
      )}
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: 12,
          padding: "7px 12px",
          backgroundColor: "var(--brush-card)",
          borderTop: "1px solid var(--brush-border)",
        }}
      >
        <span style={{ color: "var(--brush-muted)", fontSize: 12 }}>Gamma</span>
        <input
          type="text"
          className="field-box mono"
          style={{ width: 58, height: 25 }}
          value={gammaDraft}
          onChange={(e) => setGammaDraft(e.target.value)}
          onBlur={commitGammaExposure}
          onKeyDown={(e) => {
            if (e.key === "Enter") commitGammaExposure();
          }}
          title="Display gamma for the sky render (0.10 - 8.00); 1.00 leaves values unchanged"
        />

        <span style={{ color: "var(--brush-muted)", fontSize: 12 }}>Exposure</span>
        <input
          type="text"
          className="field-box mono"
          style={{ width: 58, height: 25 }}
          value={exposureDraft}
          onChange={(e) => setExposureDraft(e.target.value)}
          onBlur={commitGammaExposure}
          onKeyDown={(e) => {
            if (e.key === "Enter") commitGammaExposure();
          }}
          title="Exposure multiplier applied before gamma (0.05 - 20.00)"
        />

        <div
          className={`pill-checkbox ${autoExposure ? "checked" : ""}`}
          onClick={() => setAutoExposure(!autoExposure)}
          title="Normalize the sky to a middle-grey key (0.18) from the cubemap's own values"
        >
          <span className="pill-checkbox-box">{autoExposure && <IconCheck />}</span>
          <span>Auto exposure</span>
        </div>
      </div>
    </div>
  );
};

export interface TextureCenterViewProps {
  activeTexture: TextureNodeView;
  centerTab: string;
  mipIndex: number;
  layerIndex: number;
  opaquePreview: boolean;
  onChangeLayer: (newLayer: number) => void;
  zoom: number;
  setZoom: React.Dispatch<React.SetStateAction<number>>;
  pan: { x: number; y: number };
  setPan: React.Dispatch<React.SetStateAction<{ x: number; y: number }>>;
  isPanning: boolean;
  setIsPanning: React.Dispatch<React.SetStateAction<boolean>>;
  panStartRef: React.MutableRefObject<{ x: number; y: number }>;
  handleResetView: () => void;
  buildAssetNodeContextMenu: (nodeId: string, kind: string) => MenuItemSpec[];
  setContextMenu: React.Dispatch<React.SetStateAction<ContextMenuState | null>>;
  setHoverSubmenuIdx: React.Dispatch<React.SetStateAction<number | null>>;
  onStatus?: (msg: string) => void;
}

export const TextureCenterView: React.FC<TextureCenterViewProps> = ({
  activeTexture,
  centerTab,
  mipIndex,
  layerIndex,
  opaquePreview,
  onChangeLayer,
  zoom,
  setZoom,
  pan,
  setPan,
  isPanning,
  setIsPanning,
  panStartRef,
  handleResetView,
  buildAssetNodeContextMenu,
  setContextMenu,
  setHoverSubmenuIdx,
  onStatus,
}) => {
  const [animPlaying, setAnimPlaying] = useState(false);
  const [fpsDraft, setFpsDraft] = useState("15");
  const [fps, setFps] = useState(15);

  const isAnimated2D =
    activeTexture.dimension === "Animated 2D" && activeTexture.layer_count > 1;
  const isCubemap = activeTexture.dimension === "Cube";

  useEffect(() => {
    setAnimPlaying(false);
  }, [activeTexture.node_id]);

  useEffect(() => {
    if (!animPlaying || !isAnimated2D) return;
    const count = Math.max(1, activeTexture.layer_count);
    const intervalMs = 1000 / Math.max(1, Math.min(240, fps));
    const timer = window.setInterval(() => {
      const next = (activeTexture.current_layer + 1) % count;
      onChangeLayer(next);
    }, intervalMs);
    return () => window.clearInterval(timer);
  }, [animPlaying, isAnimated2D, activeTexture.layer_count, activeTexture.current_layer, fps, onChangeLayer]);

  const commitFps = () => {
    const parsed = parseFloat(fpsDraft.replace(",", "."));
    const clamped = Number.isFinite(parsed)
      ? Math.max(1, Math.min(240, parsed))
      : 15;
    setFps(clamped);
    setFpsDraft(String(Math.round(clamped * 100) / 100));
  };

  const copyPreviewImageToClipboard = async () => {
    setContextMenu(null);
    if (!activeTexture.preview_data_url) {
      onStatus?.("There is no decoded image to copy.");
      return;
    }
    try {
      const resp = await fetch(activeTexture.preview_data_url);
      const blob = await resp.blob();
      if (navigator.clipboard && "write" in navigator.clipboard && typeof ClipboardItem !== "undefined") {
        await navigator.clipboard.write([new ClipboardItem({ "image/png": blob })]);
        onStatus?.(`Copied Texture #${activeTexture.index} image to clipboard.`);
      } else {
        await navigator.clipboard?.writeText(activeTexture.preview_data_url);
        onStatus?.(`Copied Texture #${activeTexture.index} data URL to clipboard.`);
      }
    } catch (err) {
      onStatus?.(`Could not copy image: ${String(err)}`);
    }
  };

  return (
    <>
      {activeTexture && centerTab === "primary" ? (
        <div style={{ display: "flex", flexDirection: "column", flex: 1, minHeight: 0 }}>
          <div
            className="checkerboard-canvas"
            style={{ flex: 1, minHeight: 0 }}
            onWheel={(e) => {
              e.preventDefault();
              const factor = e.deltaY < 0 ? 1.12 : 1 / 1.12;
              setZoom((z) => Math.max(0.05, Math.min(16.0, z * factor)));
            }}
            onMouseDown={(e) => {
              if (e.button === 0) {
                setIsPanning(true);
                panStartRef.current = { x: e.clientX - pan.x, y: e.clientY - pan.y };
              }
            }}
            onMouseMove={(e) => {
              if (isPanning) {
                setPan({
                  x: e.clientX - panStartRef.current.x,
                  y: e.clientY - panStartRef.current.y,
                });
              }
            }}
            onMouseUp={() => setIsPanning(false)}
            onMouseLeave={() => setIsPanning(false)}
            onDoubleClick={handleResetView}
            onContextMenu={(e) => {
              e.preventDefault();
              e.stopPropagation();
              setHoverSubmenuIdx(null);
              const menuKind =
                activeTexture.uvd_mode === "item"
                  ? "UVD_ITEM"
                  : activeTexture.uvd_mode === "group"
                  ? "UVD_GROUP"
                  : "TEXTURE";
              const baseItems = buildAssetNodeContextMenu(activeTexture.node_id, menuKind);
              setContextMenu({
                x: Math.min(e.clientX, window.innerWidth - 260),
                y: Math.min(e.clientY, window.innerHeight - 320),
                items: [
                  {
                    label: "Reset / Fit View (100%)",
                    action: () => {
                      setContextMenu(null);
                      handleResetView();
                    },
                  },
                  {
                    label: "Copy Image to Clipboard",
                    action: () => void copyPreviewImageToClipboard(),
                  },
                  { separator: true },
                  ...baseItems,
                ],
              });
            }}
          >
            {activeTexture.preview_data_url ? (
              <img
                src={activeTexture.preview_data_url}
                alt={activeTexture.title}
                draggable={false}
                style={{
                  transform: `translate(${pan.x}px, ${pan.y}px) scale(${zoom})`,
                  transformOrigin: "center center",
                  imageRendering: zoom >= 2.0 ? "pixelated" : "auto",
                  maxWidth: "90%",
                  maxHeight: "90%",
                  objectFit: "contain",
                  pointerEvents: "none",
                }}
              />
            ) : (
              <div className="viewport-empty">
                <IconImage size={44} />
                <div className="viewport-empty-title">No preview</div>
                <div className="viewport-empty-note">{activeTexture.validation_msg}</div>
              </div>
            )}
          </div>

          {/* 2D Animated Multilayer Bar (matches XT1 Studio AnimBar) */}
          {isAnimated2D && (
            <div
              style={{
                display: "flex",
                alignItems: "center",
                gap: 10,
                padding: "6px 12px",
                backgroundColor: "var(--brush-card)",
                borderTop: "1px solid var(--brush-border)",
              }}
            >
              <button
                type="button"
                className="pill-button"
                onClick={() => setAnimPlaying((p) => !p)}
                title="Play or pause the layer animation"
              >
                <span>{animPlaying ? "⏸ Pause" : "▶ Play"}</span>
              </button>

              <span style={{ color: "var(--brush-muted)", fontSize: 12 }}>FPS</span>
              <input
                type="text"
                className="field-box mono"
                style={{ width: 48, height: 24 }}
                value={fpsDraft}
                onChange={(e) => setFpsDraft(e.target.value)}
                onBlur={commitFps}
                onKeyDown={(e) => {
                  if (e.key === "Enter") commitFps();
                }}
                title="Playback speed in frames per second (1 - 240)"
              />

              <input
                type="range"
                min={0}
                max={Math.max(1, activeTexture.layer_count - 1)}
                step={1}
                value={activeTexture.current_layer}
                style={{ flex: 1 }}
                onChange={(e) => {
                  setAnimPlaying(false);
                  onChangeLayer(Number(e.target.value));
                }}
                title="Scrub through the texture's animation frames / layers"
              />

              <span
                className="mono"
                style={{ color: "var(--brush-muted)", fontSize: 11.5, minWidth: 52, textAlign: "right" }}
              >
                {activeTexture.current_layer + 1} / {activeTexture.layer_count}
              </span>
            </div>
          )}
        </div>
      ) : activeTexture && centerTab === "secondary" ? (
        isCubemap ? (
          <CubemapSkyViewport
            nodeId={activeTexture.node_id}
            mipIndex={mipIndex}
            opaquePreview={opaquePreview}
          />
        ) : (
          <div className="viewport-empty" style={{ flex: 1 }}>
            <IconCube size={44} />
            <div className="viewport-empty-title">No cubemap preview</div>
            <div className="viewport-empty-note">
              Select a cubemap texture to orbit it in 3D here.
            </div>
          </div>
        )
      ) : null}
    </>
  );
};

export interface TextureInspectorViewProps {
  activeTexture: TextureNodeView;
  openCards: Record<string, boolean>;
  toggleCard: (key: string) => void;
  hashDraft: string;
  setHashDraft: (val: string) => void;
  flagsDraft: string;
  setFlagsDraft: (val: string) => void;
  texEncodeFormat: string;
  setTexEncodeFormat: (val: string) => void;
  texQuality: string;
  setTexQuality: (val: string) => void;
  texGenerateMips: boolean;
  setTexGenerateMips: (val: boolean) => void;
  texSolidify: boolean;
  setTexSolidify: (val: boolean) => void;
  texMakeOpaque: boolean;
  setTexMakeOpaque: (val: boolean) => void;
  texSyncFlags: boolean;
  setTexSyncFlags: (val: boolean) => void;
  runTextureMutation: (nodeId: string, action: string, payload?: any) => Promise<void>;
  promptAndRunTextureImport: (
    nodeId: string,
    action: "replace_file" | "add_texture"
  ) => Promise<void>;
  handleTextureExport: (
    nodeId: string,
    scope: "single" | "bank" | "container",
    format: "png" | "dds" | "raw"
  ) => Promise<void>;
  uvdContext?: UvdContextView | null;
  uvdActions?: UvdActions;
}

export const TextureInspectorView: React.FC<TextureInspectorViewProps> = ({
  activeTexture,
  openCards,
  toggleCard,
  hashDraft,
  setHashDraft,
  flagsDraft,
  setFlagsDraft,
  texEncodeFormat,
  setTexEncodeFormat,
  texQuality,
  setTexQuality,
  texGenerateMips,
  setTexGenerateMips,
  texSolidify,
  setTexSolidify,
  texMakeOpaque,
  setTexMakeOpaque,
  texSyncFlags,
  setTexSyncFlags,
  runTextureMutation,
  promptAndRunTextureImport,
  handleTextureExport,
  uvdContext,
  uvdActions,
}) => {
  const hashDisabled =
    activeTexture.has_confirmed_xt1_flags && !activeTexture.has_crc32_flag;
  const isUvdItem = activeTexture.uvd_mode === "item";

  return (
    <div>
      <div className="inspector-header">
        <div className="inspector-icon-box">
          <IconImage size={20} />
        </div>
        <div>
          <div className="inspector-title">{activeTexture.title}</div>
          <div className="inspector-subtitle">{activeTexture.breadcrumb}</div>
        </div>
      </div>

      <div className="inspector-spec">{activeTexture.spec_line}</div>

      <div className={`validation-banner ${activeTexture.validation_ok ? "" : "warn"}`}>
        <span>{activeTexture.validation_ok ? "✓" : "⚠"}</span>
        <span>{activeTexture.validation_msg}</span>
      </div>

      {uvdContext && uvdActions && (
        <UvdInspectorSection
          uvd={uvdContext}
          openCards={openCards}
          toggleCard={toggleCard}
          uvdActions={uvdActions}
        />
      )}

      {!isUvdItem && (
        <>
      {/* Card 1: # Hash & Built-in BNTX Name */}
      <div className="inspector-card">
        <button className="card-header-toggle" onClick={() => toggleCard("hash")}>
          <span className="card-header-left">
            <span style={{ color: "var(--brush-muted)", fontWeight: 700 }}>#</span>
            <span>Hash &amp; Game Profile</span>
          </span>
          <IconChevronDown collapsed={!openCards.hash} />
        </button>
        {openCards.hash && (
          <div className="card-body">
            <div className="inspector-row" style={{ marginBottom: 6 }}>
              <span className="inspector-row-label">Profile</span>
              <span className="mono" style={{ fontSize: 11.5 }}>
                {activeTexture.game_profile}
              </span>
            </div>

            {activeTexture.embedded_name && (
              <div className="inspector-row" style={{ marginBottom: 6 }}>
                <span className="inspector-row-label">BNTX Name</span>
                <div style={{ display: "flex", alignItems: "center", gap: 6, flex: 1 }}>
                  <span
                    className="mono"
                    style={{
                      fontSize: 11.5,
                      padding: "3px 7px",
                      borderRadius: 6,
                      backgroundColor: "var(--brush-window)",
                      border: "1px solid var(--brush-border)",
                      flex: 1,
                      overflow: "hidden",
                      textOverflow: "ellipsis",
                    }}
                    title={`Built-in BNTX BRTI texture name: ${activeTexture.embedded_name}`}
                  >
                    {activeTexture.embedded_name}
                  </span>
                  <button
                    type="button"
                    className="pill-button"
                    onClick={() =>
                      runTextureMutation(activeTexture.node_id, "calc_crc32", {
                        name: activeTexture.embedded_name,
                      })
                    }
                    title={`Recalculate CRC32 hash from embedded BNTX name '${activeTexture.embedded_name}'`}
                  >
                    <IconRefresh />
                    <span>Recalc CRC32</span>
                  </button>
                </div>
              </div>
            )}

            <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
              <span style={{ color: "var(--brush-muted)", whiteSpace: "nowrap" }}>
                CRC32 Hash
              </span>
              <input
                type="text"
                className="field-box mono"
                style={{ flex: 1, minWidth: 0, opacity: hashDisabled ? 0.5 : 1 }}
                disabled={hashDisabled}
                value={hashDraft}
                onChange={(e) => setHashDraft(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && !hashDisabled) {
                    runTextureMutation(activeTexture.node_id, "set_hash", {
                      hash: hashDraft,
                    });
                  }
                }}
                onBlur={() => {
                  if (!hashDisabled && hashDraft !== activeTexture.texture_id_hex) {
                    runTextureMutation(activeTexture.node_id, "set_hash", {
                      hash: hashDraft,
                    });
                  }
                }}
              />
              <button
                className="pill-button"
                disabled={hashDisabled}
                onClick={() => {
                  const defaultName =
                    activeTexture.embedded_name || `tex${activeTexture.index}`;
                  const inputName = window.prompt(
                    "Enter texture name to compute crc32(name) & 0x7FFFFFFF:",
                    defaultName
                  );
                  if (inputName) {
                    runTextureMutation(activeTexture.node_id, "calc_crc32", {
                      name: inputName,
                    });
                  }
                }}
                title={
                  hashDisabled
                    ? "Enable 'Has CRC32 hash' below to edit or compute the table3 hash"
                    : "Compute crc32(name) & 0x7FFFFFFF from a texture name"
                }
              >
                <IconRefresh />
                <span>Calculate CRC32</span>
              </button>
            </div>
          </div>
        )}
      </div>

      {/* Card 2: Game-Aware Flags */}
      <div className="inspector-card">
        <button className="card-header-toggle" onClick={() => toggleCard("flags")}>
          <span className="card-header-left">
            <span style={{ color: "var(--brush-muted)" }}>⚑</span>
            <span>
              {activeTexture.has_confirmed_xt1_flags
                ? "XT1 Flags (table2)"
                : "WTB Entry Flags"}
            </span>
          </span>
          <IconChevronDown collapsed={!openCards.flags} />
        </button>
        {openCards.flags && (
          <div className="card-body">
            {activeTexture.has_confirmed_xt1_flags ? (
              <>
                <div className="inspector-row">
                  <span className="inspector-row-label">Hash</span>
                  <div>
                    <div
                      className={`pill-checkbox ${
                        activeTexture.has_crc32_flag ? "checked" : ""
                      }`}
                      onClick={() =>
                        runTextureMutation(activeTexture.node_id, "toggle_xt1_flag", {
                          flag: "crc32",
                        })
                      }
                      title="Toggle table2 byte 0 bit 0x20 (LE u32 0x00000020): whether this bank entry carries a CRC32 hash in table3"
                    >
                      <span className="pill-checkbox-box">
                        {activeTexture.has_crc32_flag && <IconCheck />}
                      </span>
                      <span>Has CRC32 hash (b0: 0x20)</span>
                    </div>
                  </div>
                </div>

                <div className="inspector-row">
                  <span className="inspector-row-label">Color</span>
                  <div>
                    <div
                      className={`pill-checkbox ${
                        activeTexture.srgb_flag ? "checked" : ""
                      }`}
                      onClick={() =>
                        runTextureMutation(activeTexture.node_id, "toggle_xt1_flag", {
                          flag: "srgb",
                        })
                      }
                      title="Toggle table2 byte 3 bit 0x04 (LE u32 0x04000000): sRGB color space"
                    >
                      <span className="pill-checkbox-box">
                        {activeTexture.srgb_flag && <IconCheck />}
                      </span>
                      <span>sRGB color space (b3: 0x04)</span>
                    </div>
                  </div>
                </div>

                <div className="inspector-row">
                  <span className="inspector-row-label">Usage</span>
                  <div>
                    <div
                      className={`pill-checkbox ${
                        activeTexture.cubemap_flag ? "checked" : ""
                      }`}
                      onClick={() =>
                        runTextureMutation(activeTexture.node_id, "toggle_xt1_flag", {
                          flag: "cubemap",
                        })
                      }
                      title="Toggle table2 byte 3 bit 0x80 (LE u32 0x80000000): cubemap / environment map usage"
                    >
                      <span className="pill-checkbox-box">
                        {activeTexture.cubemap_flag && <IconCheck />}
                      </span>
                      <span>Cube map usage (b3: 0x80)</span>
                    </div>
                  </div>
                </div>
              </>
            ) : (
              <div style={{ color: "var(--brush-muted)", fontSize: 11.5, marginBottom: 6 }}>
                Raw WTB flags for {activeTexture.game_profile} (Bayonetta 3 XT1 bitmasks are isolated from non-XT1 games):
              </div>
            )}

            <div className="inspector-row">
              <span className="inspector-row-label">Flags Hex</span>
              <input
                type="text"
                className="field-box mono"
                value={flagsDraft}
                onChange={(e) => setFlagsDraft(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    runTextureMutation(activeTexture.node_id, "set_flags", {
                      flags_hex: flagsDraft,
                    });
                  }
                }}
                onBlur={() => {
                  if (flagsDraft !== activeTexture.flags_hex) {
                    runTextureMutation(activeTexture.node_id, "set_flags", {
                      flags_hex: flagsDraft,
                    });
                  }
                }}
              />
            </div>
            <div className="mono" style={{ color: "var(--brush-muted)", marginTop: 4 }}>
              table2 = {activeTexture.table2_bytes_str}
            </div>
          </div>
        )}
      </div>

      {/* Card 3: Custom Texture Import & Encoding */}
      <div className="inspector-card">
        <button className="card-header-toggle" onClick={() => toggleCard("encoding")}>
          <span className="card-header-left">
            <span style={{ color: "var(--brush-muted)" }}>⇄</span>
            <span>Import &amp; Encoding</span>
          </span>
          <IconChevronDown collapsed={!openCards.encoding} />
        </button>
        {openCards.encoding && (
          <div className="card-body">
            <div className="inspector-row">
              <span className="inspector-row-label">Format</span>
              <select
                className="pill-select mono"
                style={{ width: "100%", fontSize: 11.5 }}
                value={texEncodeFormat}
                onChange={(e) => setTexEncodeFormat(e.target.value)}
              >
                {ALL_TEXTURE_ENCODE_FORMATS.map((grp) => (
                  <optgroup key={grp.group} label={grp.group}>
                    {grp.options.map((opt) => (
                      <option key={opt.value} value={opt.value}>
                        {opt.label}
                      </option>
                    ))}
                  </optgroup>
                ))}
              </select>
            </div>

            <div className="inspector-row">
              <span className="inspector-row-label">Quality</span>
              <select
                className="pill-select"
                style={{ width: "100%", fontSize: 11.5 }}
                value={texQuality}
                onChange={(e) => setTexQuality(e.target.value)}
              >
                {ENCODE_QUALITY_OPTIONS.map((q) => (
                  <option key={q.value} value={q.value}>
                    {q.label}
                  </option>
                ))}
              </select>
            </div>

            <div className="inspector-row">
              <span className="inspector-row-label">Mipmaps</span>
              <div
                className={`pill-checkbox ${texGenerateMips ? "checked" : ""}`}
                onClick={() => setTexGenerateMips(!texGenerateMips)}
              >
                <span className="pill-checkbox-box">
                  {texGenerateMips && <IconCheck />}
                </span>
                <span>Generate full mipmap chain</span>
              </div>
            </div>

            <div className="inspector-row">
              <span className="inspector-row-label">Solidify</span>
              <div
                className={`pill-checkbox ${texSolidify ? "checked" : ""}`}
                onClick={() => setTexSolidify(!texSolidify)}
                title="Multi-source BFS alpha edge bleed (prevents dark fringes on transparent cutouts)"
              >
                <span className="pill-checkbox-box">
                  {texSolidify && <IconCheck />}
                </span>
                <span>Solidify alpha edges (BFS bleed)</span>
              </div>
            </div>

            <div className="inspector-row">
              <span className="inspector-row-label">Alpha</span>
              <div
                className={`pill-checkbox ${texMakeOpaque ? "checked" : ""}`}
                onClick={() => setTexMakeOpaque(!texMakeOpaque)}
              >
                <span className="pill-checkbox-box">
                  {texMakeOpaque && <IconCheck />}
                </span>
                <span>Force opaque alpha (A = 255)</span>
              </div>
            </div>

            {activeTexture.has_confirmed_xt1_flags && (
              <div className="inspector-row">
                <span className="inspector-row-label">Flags</span>
                <div
                  className={`pill-checkbox ${texSyncFlags ? "checked" : ""}`}
                  onClick={() => setTexSyncFlags(!texSyncFlags)}
                  title="On add or replace, sync the sRGB (b3: 0x04) and cubemap (b3: 0x80) flags from the texture format"
                >
                  <span className="pill-checkbox-box">
                    {texSyncFlags && <IconCheck />}
                  </span>
                  <span>Sync WTB sRGB / cubemap flags</span>
                </div>
              </div>
            )}

            <div
              className="mono"
              style={{
                fontSize: 10.5,
                color: "var(--brush-muted)",
                marginTop: 2,
                marginBottom: 4,
              }}
            >
              Encoder: {activeTexture.nvtt_status}
            </div>

            <div style={{ display: "flex", gap: 6, marginTop: 6 }}>
              <button
                className="pill-button accent"
                style={{ flex: 1 }}
                onClick={() =>
                  promptAndRunTextureImport(activeTexture.node_id, "replace_file")
                }
              >
                Replace Texture...
              </button>
              <button
                className="pill-button"
                onClick={() =>
                  handleTextureExport(activeTexture.node_id, "single", "png")
                }
              >
                Export PNG
              </button>
            </div>
          </div>
        )}
      </div>
        </>
      )}
    </div>
  );
};

export interface TextureImportModalProps {
  texModalState: TexModalState;
  setTexModalState: React.Dispatch<React.SetStateAction<TexModalState | null>>;
  activeTexture: TextureNodeView | null;
  setTexEncodeFormat: (val: string) => void;
  setTexQuality: (val: string) => void;
  setTexGenerateMips: (val: boolean) => void;
  setTexSolidify: (val: boolean) => void;
  setTexMakeOpaque: (val: boolean) => void;
  setTexSyncFlags: (val: boolean) => void;
  runTextureMutation: (nodeId: string, action: string, payload?: any) => Promise<void>;
}

export const TextureImportModal: React.FC<TextureImportModalProps> = ({
  texModalState,
  setTexModalState,
  activeTexture,
  setTexEncodeFormat,
  setTexQuality,
  setTexGenerateMips,
  setTexSolidify,
  setTexMakeOpaque,
  setTexSyncFlags,
  runTextureMutation,
}) => {
  return (
    <div className="modal-backdrop" onClick={() => setTexModalState(null)}>
      <div className="modal-card" onClick={(e) => e.stopPropagation()}>
        <div
          style={{
            display: "flex",
            justifyContent: "space-between",
            alignItems: "center",
          }}
        >
          <div>
            <div
              style={{
                fontSize: 15,
                fontWeight: 600,
                color: "var(--brush-heading)",
              }}
            >
              {texModalState.action === "replace_file"
                ? "Replace & Encode Texture"
                : "Add & Encode Texture to Bank"}
            </div>
            <div
              className="mono"
              style={{
                color: "var(--brush-muted)",
                fontSize: 11.5,
                marginTop: 2,
              }}
            >
              Source: {texModalState.fileName}
            </div>
          </div>
          <button
            type="button"
            className="pill-button"
            style={{ height: 26, padding: "0 8px" }}
            onClick={() => setTexModalState(null)}
          >
            ✕
          </button>
        </div>

        {activeTexture && (
          <div
            className="mono"
            style={{
              padding: "7px 10px",
              borderRadius: 8,
              backgroundColor: "var(--brush-window)",
              border: "1px solid var(--brush-border)",
              fontSize: 11.5,
              color: "var(--brush-muted)",
            }}
          >
            Target: {activeTexture.container_ext} · Current Format:{" "}
            <span style={{ color: "var(--brush-text)", fontWeight: 600 }}>
              {activeTexture.format_name}
            </span>{" "}
            ({activeTexture.width}×{activeTexture.height}, {activeTexture.mip_count}{" "}
            mips)
          </div>
        )}

        {texModalState.action === "add_texture" && (
          <div className="inspector-row">
            <span className="inspector-row-label">Texture Name</span>
            <input
              type="text"
              className="field-box mono"
              style={{ width: "100%", fontSize: 11.5 }}
              placeholder="Optional name for crc32(name) & 0x7FFFFFFF..."
              value={texModalState.textureName ?? ""}
              onChange={(e) =>
                setTexModalState({
                  ...texModalState,
                  textureName: e.target.value,
                })
              }
            />
          </div>
        )}

        <div className="inspector-row">
          <span className="inspector-row-label">Target Format</span>
          <select
            className="pill-select mono"
            style={{ width: "100%", fontSize: 11.5 }}
            value={texModalState.encodeFormat}
            onChange={(e) =>
              setTexModalState({ ...texModalState, encodeFormat: e.target.value })
            }
          >
            {ALL_TEXTURE_ENCODE_FORMATS.map((grp) => (
              <optgroup key={grp.group} label={grp.group}>
                {grp.options.map((opt) => (
                  <option key={opt.value} value={opt.value}>
                    {opt.label}
                  </option>
                ))}
              </optgroup>
            ))}
          </select>
        </div>

        <div className="inspector-row">
          <span className="inspector-row-label">Quality</span>
          <select
            className="pill-select"
            style={{ width: "100%", fontSize: 11.5 }}
            value={texModalState.quality}
            onChange={(e) =>
              setTexModalState({ ...texModalState, quality: e.target.value })
            }
          >
            {ENCODE_QUALITY_OPTIONS.map((q) => (
              <option key={q.value} value={q.value}>
                {q.label}
              </option>
            ))}
          </select>
        </div>

        <div className="inspector-row">
          <span className="inspector-row-label">Mipmaps</span>
          <div
            className={`pill-checkbox ${
              texModalState.generateMips ? "checked" : ""
            }`}
            onClick={() =>
              setTexModalState({
                ...texModalState,
                generateMips: !texModalState.generateMips,
              })
            }
          >
            <span className="pill-checkbox-box">
              {texModalState.generateMips && <IconCheck />}
            </span>
            <span>Generate full box-filtered mipmap chain</span>
          </div>
        </div>

        <div className="inspector-row">
          <span className="inspector-row-label">Transparent RGB</span>
          <div
            className={`pill-checkbox ${texModalState.solidify ? "checked" : ""}`}
            onClick={() =>
              setTexModalState({
                ...texModalState,
                solidify: !texModalState.solidify,
              })
            }
          >
            <span className="pill-checkbox-box">
              {texModalState.solidify && <IconCheck />}
            </span>
            <span>Solidify transparent edges (BFS alpha bleed)</span>
          </div>
        </div>

        <div className="inspector-row">
          <span className="inspector-row-label">Alpha</span>
          <div
            className={`pill-checkbox ${
              texModalState.makeOpaque ? "checked" : ""
            }`}
            onClick={() =>
              setTexModalState({
                ...texModalState,
                makeOpaque: !texModalState.makeOpaque,
              })
            }
          >
            <span className="pill-checkbox-box">
              {texModalState.makeOpaque && <IconCheck />}
            </span>
            <span>Force opaque alpha channel (A = 255)</span>
          </div>
        </div>

        {activeTexture?.has_confirmed_xt1_flags && (
          <div className="inspector-row">
            <span className="inspector-row-label">WTB Flags</span>
            <div
              className={`pill-checkbox ${
                texModalState.syncFlags ? "checked" : ""
              }`}
              onClick={() =>
                setTexModalState({
                  ...texModalState,
                  syncFlags: !texModalState.syncFlags,
                })
              }
            >
              <span className="pill-checkbox-box">
                {texModalState.syncFlags && <IconCheck />}
              </span>
              <span>Sync WTB sRGB / cubemap flags from format</span>
            </div>
          </div>
        )}

        <div
          style={{
            display: "flex",
            justifyContent: "flex-end",
            gap: 8,
            marginTop: 6,
          }}
        >
          <button
            type="button"
            className="pill-button"
            onClick={() => setTexModalState(null)}
          >
            Cancel
          </button>
          <button
            type="button"
            className="pill-button accent"
            onClick={async () => {
              const modal = texModalState;
              setTexModalState(null);
              setTexEncodeFormat(modal.encodeFormat);
              setTexQuality(modal.quality);
              setTexGenerateMips(modal.generateMips);
              setTexSolidify(modal.solidify);
              setTexMakeOpaque(modal.makeOpaque);
              setTexSyncFlags(modal.syncFlags);
              await runTextureMutation(modal.nodeId, modal.action, {
                path: modal.path,
                encode_format: modal.encodeFormat,
                quality: modal.quality,
                generate_mips: modal.generateMips,
                solidify: modal.solidify,
                make_opaque: modal.makeOpaque,
                sync_flags: modal.syncFlags,
                name: modal.textureName,
              });
            }}
          >
            Encode &amp; Import Texture
          </button>
        </div>
      </div>
    </div>
  );
};
