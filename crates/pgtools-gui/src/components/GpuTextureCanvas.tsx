import React, { forwardRef, useEffect, useImperativeHandle, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  channelModeToInt,
  compileGl2Program,
  decodeWtxBlob,
  formatModeForGpuFormat,
  GL2_QUAD_FS,
  GL2_QUAD_VS,
  Gl2TextureExts,
  queryGl2Exts,
  uploadSurfaceToGl2,
} from "../utils/gpuTexture";

export interface GpuTextureCanvasHandle {
  toPngBlob: () => Promise<Blob | null>;
}

export interface GpuTextureCanvasProps {
  nodeId: string;
  mipIndex: number;
  layerIndex: number;
  revision?: string;
  channelMode: string;
  opaquePreview: boolean;
  style?: React.CSSProperties;
  alt?: string;
}

export const GpuTextureCanvas = forwardRef<GpuTextureCanvasHandle, GpuTextureCanvasProps>(
  ({ nodeId, mipIndex, layerIndex, revision, channelMode, opaquePreview, style, alt }, ref) => {
    const canvasRef = useRef<HTMLCanvasElement | null>(null);
    const glStateRef = useRef<{
      gl: WebGL2RenderingContext;
      exts: Gl2TextureExts;
      program: WebGLProgram;
      tex: WebGLTexture;
      uTex: WebGLUniformLocation | null;
      uFormatMode: WebGLUniformLocation | null;
      uChannelMode: WebGLUniformLocation | null;
      uOpaque: WebGLUniformLocation | null;
      formatMode: number;
      hasTexture: boolean;
    } | null>(null);
    const [dims, setDims] = useState<{ w: number; h: number }>({ w: 1, h: 1 });
    const [uploadSeq, setUploadSeq] = useState<number>(0);
    const [errorMsg, setErrorMsg] = useState<string | null>(null);

    const draw = () => {
      const st = glStateRef.current;
      const canvas = canvasRef.current;
      if (!st || !canvas || !st.hasTexture) return;
      const { gl } = st;
      gl.viewport(0, 0, canvas.width, canvas.height);
      gl.clearColor(0, 0, 0, 0);
      gl.clear(gl.COLOR_BUFFER_BIT);
      gl.useProgram(st.program);
      gl.activeTexture(gl.TEXTURE0);
      gl.bindTexture(gl.TEXTURE_2D, st.tex);
      gl.uniform1i(st.uTex, 0);
      gl.uniform1i(st.uFormatMode, st.formatMode);
      gl.uniform1i(st.uChannelMode, channelModeToInt(channelMode));
      gl.uniform1i(st.uOpaque, opaquePreview ? 1 : 0);
      gl.drawArrays(gl.TRIANGLES, 0, 3);
    };

    useImperativeHandle(ref, () => ({
      toPngBlob: () =>
        new Promise((resolve) => {
          const canvas = canvasRef.current;
          if (!canvas) {
            resolve(null);
            return;
          }
          draw();
          canvas.toBlob((blob) => resolve(blob), "image/png");
        }),
    }));

    // Initialize WebGL2 context once per mount
    useEffect(() => {
      const canvas = canvasRef.current;
      if (!canvas) return;
      const gl = canvas.getContext("webgl2", {
        alpha: true,
        premultipliedAlpha: false,
        preserveDrawingBuffer: true,
        antialias: false,
      });
      if (!gl) {
        setErrorMsg("WebGL2 unavailable");
        return;
      }
      const program = compileGl2Program(gl, GL2_QUAD_VS, GL2_QUAD_FS);
      const tex = gl.createTexture();
      if (!program || !tex) {
        setErrorMsg("WebGL2 shader compile failed");
        return;
      }
      glStateRef.current = {
        gl,
        exts: queryGl2Exts(gl),
        program,
        tex,
        uTex: gl.getUniformLocation(program, "uTex"),
        uFormatMode: gl.getUniformLocation(program, "uFormatMode"),
        uChannelMode: gl.getUniformLocation(program, "uChannelMode"),
        uOpaque: gl.getUniformLocation(program, "uOpaque"),
        formatMode: 0,
        hasTexture: false,
      };
      return () => {
        const st = glStateRef.current;
        if (st) {
          st.gl.deleteTexture(st.tex);
          st.gl.deleteProgram(st.program);
          glStateRef.current = null;
        }
      };
    }, []);

    // Fetch WTX2 binary surface directly and upload to GPU
    useEffect(() => {
      let cancelled = false;
      setErrorMsg(null);
      (async () => {
        try {
          const raw = await invoke<ArrayBuffer>("get_texture_gpu_surface", {
            nodeId,
            mipIndex,
            layerIndex,
          });
          if (cancelled) return;
          const surface = decodeWtxBlob(raw);
          const st = glStateRef.current;
          const canvas = canvasRef.current;
          if (!st || !canvas) return;
          canvas.width = surface.width;
          canvas.height = surface.height;
          setDims({ w: surface.width, h: surface.height });
          const ok = uploadSurfaceToGl2(st.gl, st.tex, surface, st.exts);
          if (!ok) {
            setErrorMsg("GPU compressed format extension unavailable");
            return;
          }
          st.formatMode = formatModeForGpuFormat(surface.gpuFormat);
          st.hasTexture = true;
          setUploadSeq((n) => n + 1);
        } catch (err) {
          if (!cancelled) {
            setErrorMsg(String(err));
          }
        }
      })();
      return () => {
        cancelled = true;
      };
    }, [nodeId, mipIndex, layerIndex, revision]);

    // Instant GPU-only redraw when channelMode, opaquePreview, or uploaded texture changes
    useEffect(() => {
      draw();
    }, [channelMode, opaquePreview, dims.w, dims.h, uploadSeq]);

    if (errorMsg) {
      return (
        <div style={{ color: "var(--brush-warning-text)", fontSize: 12 }}>
          {errorMsg}
        </div>
      );
    }

    return (
      <canvas
        ref={canvasRef}
        aria-label={alt}
        width={dims.w}
        height={dims.h}
        style={{
          ...style,
          imageRendering: dims.w <= 64 && dims.h <= 64 ? "pixelated" : undefined,
        }}
      />
    );
  }
);

GpuTextureCanvas.displayName = "GpuTextureCanvas";
