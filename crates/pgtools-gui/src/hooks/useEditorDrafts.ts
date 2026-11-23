import React, { useCallback, useState } from "react";
import {
  BxmXmlNode,
  DecodedWmbMaterial,
  NodeInspectPayload,
  Wmb3Material,
} from "../types";
import {
  bxmTreeToXmlDocument,
  detectBxmTabular,
  parseXmlToBxmNode,
} from "../bxm/xml";

export interface EditorDraftsApi {
  // Texture
  hashDraft: string;
  setHashDraft: React.Dispatch<React.SetStateAction<string>>;
  flagsDraft: string;
  setFlagsDraft: React.Dispatch<React.SetStateAction<string>>;
  mipIndex: number;
  setMipIndex: React.Dispatch<React.SetStateAction<number>>;
  layerIndex: number;
  setLayerIndex: React.Dispatch<React.SetStateAction<number>>;
  channelMode: string;
  setChannelMode: React.Dispatch<React.SetStateAction<string>>;
  opaquePreview: boolean;
  setOpaquePreview: React.Dispatch<React.SetStateAction<boolean>>;
  texEncodeFormat: string;
  setTexEncodeFormat: React.Dispatch<React.SetStateAction<string>>;
  texQuality: string;
  setTexQuality: React.Dispatch<React.SetStateAction<string>>;
  texGenerateMips: boolean;
  setTexGenerateMips: React.Dispatch<React.SetStateAction<boolean>>;
  texSolidify: boolean;
  setTexSolidify: React.Dispatch<React.SetStateAction<boolean>>;
  texMakeOpaque: boolean;
  setTexMakeOpaque: React.Dispatch<React.SetStateAction<boolean>>;
  texSyncFlags: boolean;
  setTexSyncFlags: React.Dispatch<React.SetStateAction<boolean>>;
  zoom: number;
  setZoom: React.Dispatch<React.SetStateAction<number>>;
  pan: { x: number; y: number };
  setPan: React.Dispatch<React.SetStateAction<{ x: number; y: number }>>;
  isPanning: boolean;
  setIsPanning: React.Dispatch<React.SetStateAction<boolean>>;

  // WMB material / header
  wmbMatDraft: DecodedWmbMaterial | null;
  setWmbMatDraft: React.Dispatch<React.SetStateAction<DecodedWmbMaterial | null>>;
  wmb3MatDraft: Wmb3Material | null;
  setWmb3MatDraft: React.Dispatch<React.SetStateAction<Wmb3Material | null>>;
  wmbRawMode: boolean;
  setWmbRawMode: React.Dispatch<React.SetStateAction<boolean>>;
  wmbB2Override: boolean | null;
  setWmbB2Override: React.Dispatch<React.SetStateAction<boolean | null>>;
  wmbHeaderFlagsDraft: string;
  setWmbHeaderFlagsDraft: React.Dispatch<React.SetStateAction<string>>;
  wmbBboxDraft: number[];
  setWmbBboxDraft: React.Dispatch<React.SetStateAction<number[]>>;

  // BXM
  bxmTreeDraft: BxmXmlNode | null;
  setBxmTreeDraft: React.Dispatch<React.SetStateAction<BxmXmlNode | null>>;
  bxmXmlDraft: string;
  setBxmXmlDraft: React.Dispatch<React.SetStateAction<string>>;
  selectedBxmPath: string;
  setSelectedBxmPath: React.Dispatch<React.SetStateAction<string>>;
  bxmViewMode: "tree" | "table" | "xml";
  setBxmViewMode: React.Dispatch<React.SetStateAction<"tree" | "table" | "xml">>;
  updateBxmDraftTree: (nextRoot: BxmXmlNode) => void;
  /** Returns an error message if XML cannot be parsed when leaving XML mode. */
  switchBxmViewMode: (nextMode: "tree" | "table" | "xml") => string | null;

  // Struct / EST / CSV
  estDraft: any;
  setEstDraft: React.Dispatch<React.SetStateAction<any>>;
  csvHeadersDraft: string[];
  setCsvHeadersDraft: React.Dispatch<React.SetStateAction<string[]>>;
  csvRowsDraft: string[][];
  setCsvRowsDraft: React.Dispatch<React.SetStateAction<string[][]>>;
  structDraft: any;
  setStructDraft: React.Dispatch<React.SetStateAction<any>>;
  selectedMotTrackIdx: number;
  setSelectedMotTrackIdx: React.Dispatch<React.SetStateAction<number>>;

  // Center UI chrome
  centerTab: string;
  setCenterTab: React.Dispatch<React.SetStateAction<string>>;
  openCards: Record<string, boolean>;
  setOpenCards: (
    v: Record<string, boolean> | ((p: Record<string, boolean>) => Record<string, boolean>)
  ) => void;
  toggleCard: (key: string) => void;

  /** Apply inspect_asset_node result into the matching drafts. */
  hydrateFromInspect: (res: NodeInspectPayload) => void;
}

export function useEditorDrafts(): EditorDraftsApi {
  const [hashDraft, setHashDraft] = useState("");
  const [flagsDraft, setFlagsDraft] = useState("");
  const [mipIndex, setMipIndex] = useState(0);
  const [layerIndex, setLayerIndex] = useState(0);
  const [channelMode, setChannelMode] = useState("RGBA");
  const [opaquePreview, setOpaquePreview] = useState(false);
  const [texEncodeFormat, setTexEncodeFormat] = useState("Auto");
  const [texQuality, setTexQuality] = useState("Normal");
  const [texGenerateMips, setTexGenerateMips] = useState(true);
  const [texSolidify, setTexSolidify] = useState(true);
  const [texMakeOpaque, setTexMakeOpaque] = useState(false);
  const [texSyncFlags, setTexSyncFlags] = useState(true);
  const [zoom, setZoom] = useState(1.0);
  const [pan, setPan] = useState({ x: 0, y: 0 });
  const [isPanning, setIsPanning] = useState(false);

  const [wmbMatDraft, setWmbMatDraft] = useState<DecodedWmbMaterial | null>(null);
  const [wmb3MatDraft, setWmb3MatDraft] = useState<Wmb3Material | null>(null);
  const [wmbRawMode, setWmbRawMode] = useState(false);
  const [wmbB2Override, setWmbB2Override] = useState<boolean | null>(null);
  const [wmbHeaderFlagsDraft, setWmbHeaderFlagsDraft] = useState("");
  const [wmbBboxDraft, setWmbBboxDraft] = useState<number[]>([0, 0, 0, 0, 0, 0]);

  const [bxmTreeDraft, setBxmTreeDraft] = useState<BxmXmlNode | null>(null);
  const [bxmXmlDraft, setBxmXmlDraft] = useState("");
  const [selectedBxmPath, setSelectedBxmPath] = useState("0");
  const [bxmViewMode, setBxmViewMode] = useState<"tree" | "table" | "xml">("tree");

  const [estDraft, setEstDraft] = useState<any>(null);
  const [csvHeadersDraft, setCsvHeadersDraft] = useState<string[]>([]);
  const [csvRowsDraft, setCsvRowsDraft] = useState<string[][]>([]);
  const [structDraft, setStructDraft] = useState<any>(null);
  const [selectedMotTrackIdx, setSelectedMotTrackIdx] = useState(0);

  const [centerTab, setCenterTab] = useState("primary");
  const [openCards, setOpenCards] = useState<Record<string, boolean>>({
    hash: true,
    flags: true,
    encoding: true,
    advanced: false,
    uvd_panel: true,
    uvd_settings: true,
    wmb_props: true,
    wmb_params: true,
    bxm_node: true,
    cloth_header: true,
  });

  const updateBxmDraftTree = useCallback((nextRoot: BxmXmlNode) => {
    setBxmTreeDraft(nextRoot);
    setBxmXmlDraft(bxmTreeToXmlDocument(nextRoot));
  }, []);

  const switchBxmViewMode = useCallback(
    (nextMode: "tree" | "table" | "xml"): string | null => {
      if (bxmViewMode === "xml" && nextMode !== "xml") {
        const parsed = parseXmlToBxmNode(bxmXmlDraft);
        if (!parsed) {
          return "XML parse error: fix malformed XML syntax before switching to Tree/Table view";
        }
        setBxmTreeDraft(parsed);
      } else if (bxmViewMode !== "xml" && nextMode === "xml" && bxmTreeDraft) {
        setBxmXmlDraft(bxmTreeToXmlDocument(bxmTreeDraft));
      }
      setBxmViewMode(nextMode);
      return null;
    },
    [bxmViewMode, bxmXmlDraft, bxmTreeDraft]
  );

  const hydrateFromInspect = useCallback((res: NodeInspectPayload) => {
    if (res.view_type === "TEXTURE" && res.texture) {
      const t = res.texture;
      setHashDraft(t.texture_id_hex);
      setFlagsDraft(t.flags_hex);
    } else if (res.view_type === "WMB" && res.wmb) {
      const w = res.wmb;
      if (w.material_decoded) {
        const norm: DecodedWmbMaterial = {
          ...w.material_decoded,
          fields: Array.isArray(w.material_decoded.fields)
            ? w.material_decoded.fields
            : [],
          raw_dwords: Array.isArray(w.material_decoded.raw_dwords)
            ? w.material_decoded.raw_dwords
            : [],
        };
        setWmbMatDraft(norm);
      } else {
        setWmbMatDraft(null);
      }
      if (w.wmb3_material) {
        setWmb3MatDraft(structuredClone(w.wmb3_material));
      } else {
        setWmb3MatDraft(null);
      }
      if (w.section === "header" && w.data) {
        const vf = w.data.vertex_flags;
        if (typeof vf === "number") {
          setWmbHeaderFlagsDraft(
            `0x${(vf >>> 0).toString(16).toUpperCase().padStart(8, "0")}`
          );
        }
        if (Array.isArray(w.data.bounding_box) && w.data.bounding_box.length === 6) {
          setWmbBboxDraft([...w.data.bounding_box]);
        }
      }
    } else if (res.view_type === "BXM") {
      const clonedRoot: BxmXmlNode | null = res.root
        ? structuredClone(res.root)
        : res.bxm
          ? structuredClone(res.bxm)
          : null;
      setBxmTreeDraft(clonedRoot);
      setBxmXmlDraft(
        (res as any).xml ??
          (clonedRoot ? bxmTreeToXmlDocument(clonedRoot) : "")
      );
      setSelectedBxmPath("0");
      if (detectBxmTabular(clonedRoot)) {
        setBxmViewMode("table");
      } else {
        setBxmViewMode("tree");
      }
    } else if (res.view_type === "EST" && res.est) {
      setEstDraft(structuredClone(res.est));
    } else if (res.view_type === "CSV") {
      setCsvHeadersDraft(
        Array.isArray((res as any).headers) ? [...(res as any).headers] : []
      );
      setCsvRowsDraft(
        Array.isArray((res as any).rows)
          ? structuredClone((res as any).rows)
          : []
      );
    } else if (res.view_type === "RAW_STRUCT") {
      setStructDraft(structuredClone(res.data));
      setSelectedMotTrackIdx(0);
    }
  }, []);

  const toggleCard = useCallback((key: string) => {
    setOpenCards((prev) => ({ ...prev, [key]: !prev[key] }));
  }, []);

  return {
    hashDraft,
    setHashDraft,
    flagsDraft,
    setFlagsDraft,
    mipIndex,
    setMipIndex,
    layerIndex,
    setLayerIndex,
    channelMode,
    setChannelMode,
    opaquePreview,
    setOpaquePreview,
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
    zoom,
    setZoom,
    pan,
    setPan,
    isPanning,
    setIsPanning,
    wmbMatDraft,
    setWmbMatDraft,
    wmb3MatDraft,
    setWmb3MatDraft,
    wmbRawMode,
    setWmbRawMode,
    wmbB2Override,
    setWmbB2Override,
    wmbHeaderFlagsDraft,
    setWmbHeaderFlagsDraft,
    wmbBboxDraft,
    setWmbBboxDraft,
    bxmTreeDraft,
    setBxmTreeDraft,
    bxmXmlDraft,
    setBxmXmlDraft,
    selectedBxmPath,
    setSelectedBxmPath,
    bxmViewMode,
    setBxmViewMode,
    updateBxmDraftTree,
    switchBxmViewMode,
    estDraft,
    setEstDraft,
    csvHeadersDraft,
    setCsvHeadersDraft,
    csvRowsDraft,
    setCsvRowsDraft,
    structDraft,
    setStructDraft,
    selectedMotTrackIdx,
    setSelectedMotTrackIdx,
    centerTab,
    setCenterTab,
    openCards,
    setOpenCards,
    toggleCard,
    hydrateFromInspect,
  };
}
