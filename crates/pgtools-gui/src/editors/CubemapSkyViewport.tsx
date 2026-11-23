import React, { useEffect, useRef, useState } from "react";
import { getCubemapFacesPreview, getTextureGpuCubemap } from "../api/tauri";
import { StudioRange } from "../components/StudioRange";
import {
  channelModeToInt,
  compileGl2Program,
  decodeWtxBlob,
  formatModeForGpuFormat,
  GL2_CUBEMAP_SKY_FS,
  GL2_CUBEMAP_SKY_VS,
  Gl2TextureExts,
  queryGl2Exts,
  uploadCubemapToGl2,
} from "../utils/gpuTexture";
import { IconCheck } from "../components/Icons";

interface CubemapSkyViewportProps {
  nodeId: string;
  mipIndex: number;
  channelMode: string;
  opaquePreview: boolean;
  revision?: string;
}

export const CubemapSkyViewport: React.FC<CubemapSkyViewportProps> = ({
  nodeId,
  mipIndex,
  channelMode,
  opaquePreview,
  revision,
}) => {
  const mountRef = useRef<HTMLDivElement | null>(null);
  const [gammaDraft, setGammaDraft] = useState("1.00");
  const [exposureDraft, setExposureDraft] = useState("1.00");
  const [gamma, setGamma] = useState(1.0);
  const [exposure, setExposure] = useState(1.0);
  const [autoExposure, setAutoExposure] = useState(true);
  const [autoScale, setAutoScale] = useState(1.0);
  const [loadingErr, setLoadingErr] = useState<string | null>(null);

  const glStateRef = useRef<{
    canvas: HTMLCanvasElement;
    gl: WebGL2RenderingContext;
    exts: Gl2TextureExts;
    program: WebGLProgram;
    tex: WebGLTexture;
    uCube: WebGLUniformLocation | null;
    uForward: WebGLUniformLocation | null;
    uRight: WebGLUniformLocation | null;
    uUp: WebGLUniformLocation | null;
    uAspect: WebGLUniformLocation | null;
    uTanHalfFov: WebGLUniformLocation | null;
    uFormatMode: WebGLUniformLocation | null;
    uChannelMode: WebGLUniformLocation | null;
    uOpaque: WebGLUniformLocation | null;
    uExposure: WebGLUniformLocation | null;
    uGamma: WebGLUniformLocation | null;
    formatMode: number;
    hasTexture: boolean;
  } | null>(null);

  const viewStateRef = useRef({
    yaw: 0,
    pitch: 0,
    fov: 60,
    exposure: 1.0,
    baseExposure: 1.0,
    autoExposure: true,
    gamma: 1.0,
    channelMode: 0,
    opaque: 0,
  });

  const drawSky = (
    overrideExp?: number,
    overrideGamma?: number,
    overrideChannel?: number,
    overrideViewportSize?: number
  ) => {
    const st = glStateRef.current;
    if (!st || !st.hasTexture) return;
    const { gl, canvas } = st;
    const w = overrideViewportSize ?? Math.max(1, canvas.width);
    const h = overrideViewportSize ?? Math.max(1, canvas.height);
    const vs = viewStateRef.current;

    const phi = ((90 - vs.pitch) * Math.PI) / 180;
    const theta = (vs.yaw * Math.PI) / 180;
    const fx = Math.sin(phi) * Math.cos(theta);
    const fy = Math.cos(phi);
    const fz = Math.sin(phi) * Math.sin(theta);

    // right = normalize(cross(forward, (0, 1, 0))) = normalize((-fz, 0, fx))
    let rx = -fz;
    let ry = 0;
    let rz = fx;
    const rLen = Math.hypot(rx, ry, rz) || 1;
    rx /= rLen;
    rz /= rLen;

    // up = cross(right, forward)
    const ux = ry * fz - rz * fy;
    const uy = rz * fx - rx * fz;
    const uz = rx * fy - ry * fx;

    const tanHalfFov = Math.tan(((vs.fov * 0.5) * Math.PI) / 180);

    gl.viewport(0, 0, w, h);
    gl.clearColor(0.08, 0.08, 0.1, 1.0);
    gl.clear(gl.COLOR_BUFFER_BIT);
    gl.useProgram(st.program);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_CUBE_MAP, st.tex);
    gl.uniform1i(st.uCube, 0);
    gl.uniform3f(st.uForward, fx, fy, fz);
    gl.uniform3f(st.uRight, rx, ry, rz);
    gl.uniform3f(st.uUp, ux, uy, uz);
    gl.uniform1f(st.uAspect, w / h);
    gl.uniform1f(st.uTanHalfFov, tanHalfFov);
    gl.uniform1i(st.uFormatMode, st.formatMode);
    gl.uniform1i(st.uChannelMode, overrideChannel ?? vs.channelMode);
    gl.uniform1i(st.uOpaque, vs.opaque);
    gl.uniform1f(st.uExposure, overrideExp ?? vs.exposure);
    gl.uniform1f(st.uGamma, Math.max(0.1, overrideGamma ?? vs.gamma));
    gl.drawArrays(gl.TRIANGLES, 0, 3);
  };

  const estimateAutoExposureFromGl = () => {
    const st = glStateRef.current;
    if (!st || !st.hasTexture) return 1.0;
    const { gl, canvas } = st;
    const grid = Math.min(8, Math.max(1, canvas.width), Math.max(1, canvas.height));
    const isHdr = st.formatMode === 3;
    const savedYaw = viewStateRef.current.yaw;
    const savedPitch = viewStateRef.current.pitch;
    const savedFov = viewStateRef.current.fov;

    // Render 6 square 90 deg faces into a tiny grid x grid viewport (6 readbacks instead of 384)
    const dirs: Array<[number, number]> = [
      [0, 0],
      [90, 0],
      [180, 0],
      [270, 0],
      [0, 89],
      [0, -89],
    ];
    const buf = new Uint8Array(grid * grid * 4);
    let logSum = 0;
    let count = 0;
    viewStateRef.current.fov = 90;
    for (const [dyaw, dpitch] of dirs) {
      viewStateRef.current.yaw = dyaw;
      viewStateRef.current.pitch = dpitch;
      drawSky(1.0, 1.0, 0, grid);
      gl.readPixels(0, 0, grid, grid, gl.RGBA, gl.UNSIGNED_BYTE, buf);
      for (let i = 0; i < buf.length; i += 4) {
        let r = buf[i] / 255.0;
        let g = buf[i + 1] / 255.0;
        let b = buf[i + 2] / 255.0;
        if (isHdr) {
          r = r / Math.max(0.004, 1.0 - r);
          g = g / Math.max(0.004, 1.0 - g);
          b = b / Math.max(0.004, 1.0 - b);
        }
        const lum = Math.max(1e-4, 0.2126 * r + 0.7152 * g + 0.0722 * b);
        logSum += Math.log(lum);
        count++;
      }
    }
    viewStateRef.current.yaw = savedYaw;
    viewStateRef.current.pitch = savedPitch;
    viewStateRef.current.fov = savedFov;
    if (count === 0) return 1.0;
    const scale = 0.18 / Math.exp(logSum / count);
    if (!Number.isFinite(scale)) return 1.0;
    return Math.max(1 / 32, Math.min(32, scale));
  };

  // Initialize direct WebGL2 cubemap skybox context once per mount
  useEffect(() => {
    const host = mountRef.current;
    if (!host) return;

    const canvas = document.createElement("canvas");
    canvas.style.width = "100%";
    canvas.style.height = "100%";
    canvas.style.display = "block";
    host.innerHTML = "";
    host.appendChild(canvas);

    const syncSize = () => {
      const dpr = Math.min(window.devicePixelRatio || 1, 2);
      const w = Math.max(1, Math.round(host.clientWidth * dpr));
      const h = Math.max(1, Math.round(host.clientHeight * dpr));
      if (canvas.width !== w || canvas.height !== h) {
        canvas.width = w;
        canvas.height = h;
      }
    };
    syncSize();

    const gl = canvas.getContext("webgl2", {
      alpha: false,
      antialias: true,
      preserveDrawingBuffer: false,
    });
    if (!gl) {
      setLoadingErr("WebGL2 unavailable");
      return;
    }
    const program = compileGl2Program(gl, GL2_CUBEMAP_SKY_VS, GL2_CUBEMAP_SKY_FS);
    const tex = gl.createTexture();
    if (!program || !tex) {
      setLoadingErr("WebGL2 cubemap shader compile failed");
      return;
    }

    glStateRef.current = {
      canvas,
      gl,
      exts: queryGl2Exts(gl),
      program,
      tex,
      uCube: gl.getUniformLocation(program, "uCube"),
      uForward: gl.getUniformLocation(program, "uForward"),
      uRight: gl.getUniformLocation(program, "uRight"),
      uUp: gl.getUniformLocation(program, "uUp"),
      uAspect: gl.getUniformLocation(program, "uAspect"),
      uTanHalfFov: gl.getUniformLocation(program, "uTanHalfFov"),
      uFormatMode: gl.getUniformLocation(program, "uFormatMode"),
      uChannelMode: gl.getUniformLocation(program, "uChannelMode"),
      uOpaque: gl.getUniformLocation(program, "uOpaque"),
      uExposure: gl.getUniformLocation(program, "uExposure"),
      uGamma: gl.getUniformLocation(program, "uGamma"),
      formatMode: 0,
      hasTexture: false,
    };

    let dragging = false;
    let lastX = 0;
    let lastY = 0;

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
      viewStateRef.current.yaw -= dx * 0.25;
      viewStateRef.current.pitch = Math.max(
        -89,
        Math.min(89, viewStateRef.current.pitch + dy * 0.25)
      );
      drawSky();
    };
    const onMouseUp = () => {
      dragging = false;
    };
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      viewStateRef.current.fov = Math.max(
        20,
        Math.min(100, viewStateRef.current.fov + (e.deltaY > 0 ? 4 : -4))
      );
      drawSky();
    };
    const onDblClick = () => {
      viewStateRef.current.yaw = 0;
      viewStateRef.current.pitch = 0;
      viewStateRef.current.fov = 60;
      drawSky();
    };

    canvas.addEventListener("mousedown", onMouseDown);
    window.addEventListener("mousemove", onMouseMove);
    window.addEventListener("mouseup", onMouseUp);
    canvas.addEventListener("wheel", onWheel, { passive: false });
    canvas.addEventListener("dblclick", onDblClick);

    const ro = new ResizeObserver(() => {
      syncSize();
      drawSky();
    });
    ro.observe(host);

    return () => {
      ro.disconnect();
      canvas.removeEventListener("mousedown", onMouseDown);
      window.removeEventListener("mousemove", onMouseMove);
      window.removeEventListener("mouseup", onMouseUp);
      canvas.removeEventListener("wheel", onWheel);
      canvas.removeEventListener("dblclick", onDblClick);
      const st = glStateRef.current;
      if (st) {
        st.gl.deleteTexture(st.tex);
        st.gl.deleteProgram(st.program);
        glStateRef.current = null;
      }
    };
  }, []);

  useEffect(() => {
    const effExp = exposure * (autoExposure ? autoScale : 1.0);
    viewStateRef.current.baseExposure = exposure;
    viewStateRef.current.autoExposure = autoExposure;
    viewStateRef.current.exposure = effExp;
    viewStateRef.current.gamma = Math.max(0.1, gamma);
    viewStateRef.current.channelMode = channelModeToInt(channelMode);
    viewStateRef.current.opaque = opaquePreview ? 1 : 0;
    drawSky();
  }, [exposure, gamma, autoExposure, autoScale, channelMode, opaquePreview]);

  useEffect(() => {
    let cancelled = false;
    setLoadingErr(null);
    if (glStateRef.current) {
      glStateRef.current.hasTexture = false;
      const { gl, canvas } = glStateRef.current;
      gl.viewport(0, 0, Math.max(1, canvas.width), Math.max(1, canvas.height));
      gl.clearColor(0.08, 0.08, 0.1, 1.0);
      gl.clear(gl.COLOR_BUFFER_BIT);
    }

    const loadHtmlImage = (url: string) =>
      new Promise<HTMLImageElement>((resolve, reject) => {
        const img = new Image();
        img.onload = () => resolve(img);
        img.onerror = (e) => reject(e);
        img.src = url;
      });

    (async () => {
      try {
        // Primary path: direct GPU-compressed cubemap upload (matches Asset Browser thumbnails, true HDR for BC6H)
        let uploadedDirect = false;
        try {
          const raw = await getTextureGpuCubemap({
            nodeId,
            mipIndex,
          });
          if (cancelled) return;
          const surface = decodeWtxBlob(raw);
          const st = glStateRef.current;
          if (st && uploadCubemapToGl2(st.gl, st.tex, surface, st.exts)) {
            st.formatMode = formatModeForGpuFormat(surface.gpuFormat);
            st.hasTexture = true;
            uploadedDirect = true;
            const estScale = estimateAutoExposureFromGl();
            if (!cancelled) {
              setAutoScale(estScale);
              const vs = viewStateRef.current;
              vs.exposure = vs.baseExposure * (vs.autoExposure ? estScale : 1.0);
              drawSky();
            }
          }
        } catch {
          uploadedDirect = false;
        }

        if (uploadedDirect || cancelled) return;

        // Fallback path: CPU-decoded 6 face PNGs (preserve alpha so uChannelMode/uOpaque work live on GPU)
        const res = await getCubemapFacesPreview({
          nodeId,
          mipIndex,
          opaque: false,
        });
        if (cancelled) return;
        const images = await Promise.all(res.face_data_urls.map(loadHtmlImage));
        if (cancelled) return;
        const st = glStateRef.current;
        if (!st || images.length < 6) return;
        const { gl } = st;
        gl.bindTexture(gl.TEXTURE_CUBE_MAP, st.tex);
        gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
        gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, 0);
        gl.texParameteri(gl.TEXTURE_CUBE_MAP, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
        gl.texParameteri(gl.TEXTURE_CUBE_MAP, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
        gl.texParameteri(gl.TEXTURE_CUBE_MAP, gl.TEXTURE_WRAP_R, gl.CLAMP_TO_EDGE);
        gl.texParameteri(gl.TEXTURE_CUBE_MAP, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
        gl.texParameteri(gl.TEXTURE_CUBE_MAP, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
        for (let f = 0; f < 6; f++) {
          gl.texImage2D(
            gl.TEXTURE_CUBE_MAP_POSITIVE_X + f,
            0,
            gl.RGBA,
            gl.RGBA,
            gl.UNSIGNED_BYTE,
            images[f]
          );
        }
        st.formatMode = 0;
        st.hasTexture = true;
        const scale = res.auto_exposure_scale || 1.0;
        setAutoScale(scale);
        const vs = viewStateRef.current;
        vs.exposure = vs.baseExposure * (vs.autoExposure ? scale : 1.0);
        drawSky();
      } catch (err) {
        if (!cancelled) {
          if (glStateRef.current) {
            glStateRef.current.hasTexture = false;
            const { gl, canvas } = glStateRef.current;
            gl.viewport(0, 0, Math.max(1, canvas.width), Math.max(1, canvas.height));
            gl.clearColor(0.08, 0.08, 0.1, 1.0);
            gl.clear(gl.COLOR_BUFFER_BIT);
          }
          setLoadingErr(String(err));
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [nodeId, mipIndex, revision]);

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
        data-tooltip="Drag to orbit cubemap sky, wheel to zoom, double-click to reset camera"
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
          data-tooltip="Display gamma for the sky render (0.10 - 8.00); 1.00 leaves values unchanged"
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
          data-tooltip="Exposure multiplier applied before gamma (0.05 - 20.00)"
        />

        <div
          className={`pill-checkbox ${autoExposure ? "checked" : ""}`}
          onClick={() => setAutoExposure(!autoExposure)}
          data-tooltip="Normalize the sky to a middle-grey key (0.18) from the cubemap's own values"
        >
          <span className="pill-checkbox-box">{autoExposure && <IconCheck />}</span>
          <span>Auto exposure</span>
        </div>
      </div>
    </div>
  );
};

