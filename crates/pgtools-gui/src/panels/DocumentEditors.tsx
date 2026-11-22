import React from "react";
import {
  ContextMenuState,
  DocumentOutline,
  MaterialClipboardState,
  MenuItemSpec,
  NodeInspectPayload,
  PointerDragState,
  TextureNodeView,
  WmbSectionView,
} from "../types";
import { BxmCenterView, BxmInspectorView } from "../bxm/BxmEditor";
import {
  TextureCenterView,
  TextureInspectorView,
} from "../editors/TextureEditor";
import {
  UvdActions,
  UvdCenterView,
  UvdInspectorSection,
} from "../editors/UvdEditor";
import { ClothInspectorView, StructCenterView } from "../editors/StructEditors";
import {
  WmbInspectorView,
  WmbStructuredCenterView,
} from "../editors/WmbStructuredEditor";
import {
  IconChevronDown,
  IconCode,
  IconCube,
  IconImage,
} from "../components/Icons";
import { EditorDraftsApi } from "../hooks/useEditorDrafts";
import { MeshUiApi } from "../hooks/useMeshUi";

export interface DocumentSelection {
  nodePayload: NodeInspectPayload | null;
  setNodePayload: React.Dispatch<React.SetStateAction<NodeInspectPayload | null>>;
  activeTexture: TextureNodeView | null;
  activeWmb: WmbSectionView | null;
  outline: DocumentOutline | null;
  assetSearch: string;
}

export interface DocumentLayout {
  rightWidth: number;
  draggingSplit: "left" | "right" | "left-row" | null;
  setDraggingSplit: (v: "left" | "right" | "left-row" | null) => void;
  setRightWidth: (n: number) => void;
  panStartRef: React.MutableRefObject<{ x: number; y: number }>;
}

export interface DocumentDllSettings {
  oodleDllPath: string;
  setOodleDllPath: (s: string) => void;
  oodleConfigured?: boolean;
  nvttDllPath: string;
  setNvttDllPath: (s: string) => void;
  nvttConfigured?: boolean;
  handlePickOodleDll: () => Promise<void>;
  handlePickNvttDll: () => Promise<void>;
}

export interface DocumentActions {
  selectAndInspectNode: (...args: any[]) => Promise<void>;
  runWmbMutation: (nodeId: string, operation: string, payload?: any) => Promise<void>;
  runTextureMutation: (nodeId: string, action: string, payload?: any) => Promise<void>;
  runStructMutation: (nodeId: string, structKind: string, updatedData: any) => Promise<void>;
  runClothMutation: (
    nodeId: string,
    operation: string,
    preset?: string,
    updatedJson?: any
  ) => Promise<void>;
  runContainerMutation: (
    nodeId: string,
    action: string,
    targetPath?: string,
    payload?: any
  ) => Promise<void>;
  openPkzCompressionModal: (
    title: string,
    subtitle: string,
    initialCodec: string | undefined,
    onConfirm: (
      compression: "None" | "ZStandard" | "OodleKraken",
      level: number,
      forceCompression: boolean
    ) => Promise<void> | void
  ) => void;
  handleCrossGameMaterialConvert: (
    wmb: WmbSectionView,
    dstGame: "b1" | "b2" | "b3"
  ) => Promise<void>;
  promptAndRunTextureImport: (
    nodeId: string,
    action: "replace_file" | "add_texture"
  ) => Promise<void>;
  handleTextureExport: (
    nodeId: string,
    scope: "single" | "bank" | "container",
    format: "png" | "dds" | "raw"
  ) => Promise<void>;
  handleResetView: () => void;
  commitBxmDraft: (customXml?: string) => Promise<void>;
  loadDirectory: (target?: string) => Promise<void>;
  currentDirRef: React.MutableRefObject<string>;
  buildAssetNodeContextMenu: (nodeId: string, kind: string) => MenuItemSpec[];
  setContextMenu: React.Dispatch<React.SetStateAction<ContextMenuState | null>>;
  setHoverSubmenuIdx: React.Dispatch<React.SetStateAction<number | null>>;
  setStatusLeft: (msg: string) => void;
  switchBxmViewMode: (m: "tree" | "table" | "xml") => void;
  uvdActions: UvdActions;
}

export interface DocumentEditorsProps {
  selection: DocumentSelection;
  drafts: EditorDraftsApi;
  meshUi: MeshUiApi;
  layout: DocumentLayout;
  dll: DocumentDllSettings;
  actions: DocumentActions;
  materialClipboard: MaterialClipboardState | null;
  setMaterialClipboard: React.Dispatch<React.SetStateAction<MaterialClipboardState | null>>;
  pointerDrag: PointerDragState | null;
  setPointerDrag: React.Dispatch<React.SetStateAction<PointerDragState | null>>;
}

/**
 * Center viewport + right inspector, switched by the active node payload.
 */
export function DocumentEditors({
  selection,
  drafts,
  meshUi,
  layout,
  dll,
  actions,
  materialClipboard,
  setMaterialClipboard,
  pointerDrag,
  setPointerDrag,
}: DocumentEditorsProps) {
  const {
    nodePayload,
    setNodePayload,
    activeTexture,
    activeWmb,
    outline,
    assetSearch,
  } = selection;

  const {
    centerTab,
    setCenterTab,
    bxmViewMode,
    zoom,
    setZoom,
    pan,
    setPan,
    isPanning,
    setIsPanning,
    mipIndex,
    layerIndex,
    opaquePreview,
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
    bxmXmlDraft,
    setBxmXmlDraft,
    selectedBxmPath,
    setSelectedBxmPath,
    bxmExpandedPaths,
    setBxmExpandedPaths,
    updateBxmDraftTree,
    csvHeadersDraft,
    setCsvHeadersDraft,
    csvRowsDraft,
    setCsvRowsDraft,
    estDraft,
    setEstDraft,
    structDraft,
    setStructDraft,
    selectedMotTrackIdx,
    setSelectedMotTrackIdx,
    openCards,
    toggleCard,
  } = drafts;

  const {
    rightWidth,
    draggingSplit,
    setDraggingSplit,
    setRightWidth,
    panStartRef,
  } = layout;

  return (
    <>
      <div className="center-column">
        <div className="studio-card viewer-card">
          <div className="underline-tabbar">
            {activeTexture ? (
              <button className="underline-tab active" type="button">
                {activeTexture.dimension === "Cube" && activeTexture.uvd_mode !== "item" ? (
                  <>
                    <IconCube size={12} />
                    <span>Cube-3D</span>
                  </>
                ) : (
                  <>
                    <IconImage size={12} />
                    <span>{activeTexture.uvd_mode === "item" ? "Sprite Crop" : "2D"}</span>
                  </>
                )}
              </button>
            ) : nodePayload?.view_type === "UVD" ? (
              <button className="underline-tab active" onClick={() => setCenterTab("primary")}>
                <IconImage size={12} />
                <span>UVD Atlas Groups</span>
              </button>
            ) : nodePayload?.view_type === "BXM" ? (
              <>
                <button
                  className={`underline-tab ${bxmViewMode === "tree" ? "active" : ""}`}
                  onClick={() => actions.switchBxmViewMode("tree")}
                >
                  <IconCode size={12} />
                  <span>Interactive Tree</span>
                </button>
                <button
                  className={`underline-tab ${bxmViewMode === "table" ? "active" : ""}`}
                  onClick={() => actions.switchBxmViewMode("table")}
                >
                  <IconCube size={12} />
                  <span>Table View</span>
                </button>
                <button
                  className={`underline-tab ${bxmViewMode === "xml" ? "active" : ""}`}
                  onClick={() => actions.switchBxmViewMode("xml")}
                >
                  <IconCode size={12} />
                  <span>Raw XML</span>
                </button>
              </>
            ) : (
              <>
                <button
                  className={`underline-tab ${centerTab === "primary" ? "active" : ""}`}
                  onClick={() => setCenterTab("primary")}
                >
                  <IconCube size={12} />
                  <span>Structured Editor</span>
                </button>
                <button
                  className={`underline-tab ${centerTab === "secondary" ? "active" : ""}`}
                  onClick={() => setCenterTab("secondary")}
                >
                  <IconCode size={12} />
                  <span>Hex / Raw Inspector</span>
                </button>
              </>
            )}
          </div>

          {activeTexture ? (
            <TextureCenterView
              activeTexture={activeTexture}
              mipIndex={mipIndex}
              layerIndex={layerIndex}
              channelMode={drafts.channelMode}
              opaquePreview={opaquePreview}
              onChangeLayer={(newLayer) => {
                drafts.setLayerIndex(newLayer);
                if (!activeTexture.gpu_direct) {
                  void actions.selectAndInspectNode(
                    activeTexture.node_id,
                    mipIndex,
                    drafts.channelMode,
                    opaquePreview,
                    wmbB2Override,
                    undefined,
                    newLayer
                  );
                }
              }}
              zoom={zoom}
              setZoom={setZoom}
              pan={pan}
              setPan={setPan}
              isPanning={isPanning}
              setIsPanning={setIsPanning}
              panStartRef={panStartRef}
              handleResetView={actions.handleResetView}
              buildAssetNodeContextMenu={actions.buildAssetNodeContextMenu}
              setContextMenu={actions.setContextMenu}
              setHoverSubmenuIdx={actions.setHoverSubmenuIdx}
              onStatus={actions.setStatusLeft}
            />
          ) : nodePayload?.view_type === "UVD" && nodePayload.uvd ? (
            <UvdCenterView
              uvd={nodePayload.uvd}
              specLine={nodePayload.spec_line}
              uvdActions={actions.uvdActions}
            />
          ) : activeWmb && centerTab === "primary" ? (
            <div
              className="struct-table-container"
              onContextMenu={(e) => {
                e.preventDefault();
                e.stopPropagation();
                actions.setHoverSubmenuIdx(null);
                const items = actions.buildAssetNodeContextMenu(
                  activeWmb.node_id,
                  activeWmb.is_wmb3 ? "WMB3" : "WMB"
                );
                if (items.length > 0) {
                  actions.setContextMenu({
                    x: Math.min(e.clientX, window.innerWidth - 250),
                    y: Math.min(e.clientY, window.innerHeight - 260),
                    items,
                  });
                }
              }}
            >
              <WmbStructuredCenterView
                wmb={activeWmb}
                wmbMatDraft={wmbMatDraft}
                setWmbMatDraft={setWmbMatDraft}
                wmb3MatDraft={wmb3MatDraft}
                setWmb3MatDraft={setWmb3MatDraft}
                wmbRawMode={wmbRawMode}
                setWmbRawMode={setWmbRawMode}
                wmbB2Override={wmbB2Override}
                setWmbB2Override={setWmbB2Override}
                materialClipboard={materialClipboard}
                setMaterialClipboard={setMaterialClipboard}
                wmbHeaderFlagsDraft={wmbHeaderFlagsDraft}
                setWmbHeaderFlagsDraft={setWmbHeaderFlagsDraft}
                wmbBboxDraft={wmbBboxDraft}
                setWmbBboxDraft={setWmbBboxDraft}
                expandedMeshGroups={meshUi.expandedMeshGroups}
                setExpandedMeshGroups={meshUi.setExpandedMeshGroups}
                expandedBatchLods={meshUi.expandedBatchLods}
                setExpandedBatchLods={meshUi.setExpandedBatchLods}
                draggedBatchInfo={meshUi.draggedBatchInfo}
                setDraggedBatchInfo={meshUi.setDraggedBatchInfo}
                dragOverMeshIdx={meshUi.dragOverMeshIdx}
                setDragOverMeshIdx={meshUi.setDragOverMeshIdx}
                selectAndInspectNode={actions.selectAndInspectNode}
                runWmbMutation={actions.runWmbMutation}
                handleCrossGameMaterialConvert={actions.handleCrossGameMaterialConvert}
                setStatusLeft={actions.setStatusLeft}
              />
            </div>
          ) : nodePayload?.view_type === "BXM" ? (
            <BxmCenterView
              nodePayload={nodePayload}
              bxmViewMode={bxmViewMode}
              bxmTreeDraft={bxmTreeDraft}
              bxmXmlDraft={bxmXmlDraft}
              setBxmXmlDraft={setBxmXmlDraft}
              selectedBxmPath={selectedBxmPath}
              setSelectedBxmPath={setSelectedBxmPath}
              bxmExpandedPaths={bxmExpandedPaths}
              setBxmExpandedPaths={setBxmExpandedPaths}
              pointerDrag={pointerDrag}
              setPointerDrag={setPointerDrag}
              updateBxmDraftTree={updateBxmDraftTree}
              commitBxmDraft={actions.commitBxmDraft}
              runContainerMutation={actions.runContainerMutation}
              setContextMenu={actions.setContextMenu}
              setHoverSubmenuIdx={actions.setHoverSubmenuIdx}
              setStatusLeft={actions.setStatusLeft}
            />
          ) : nodePayload ? (
            <StructCenterView
              nodePayload={nodePayload}
              setNodePayload={setNodePayload}
              centerTab={centerTab}
              assetSearch={assetSearch}
              csvHeadersDraft={csvHeadersDraft}
              setCsvHeadersDraft={setCsvHeadersDraft}
              csvRowsDraft={csvRowsDraft}
              setCsvRowsDraft={setCsvRowsDraft}
              estDraft={estDraft}
              setEstDraft={setEstDraft}
              structDraft={structDraft}
              setStructDraft={setStructDraft}
              selectedMotTrackIdx={selectedMotTrackIdx}
              setSelectedMotTrackIdx={setSelectedMotTrackIdx}
              pkzBulkCodec={meshUi.pkzBulkCodec}
              setPkzBulkCodec={meshUi.setPkzBulkCodec}
              pkzSortCol={meshUi.pkzSortCol}
              pkzSortAsc={meshUi.pkzSortAsc}
              togglePkzSort={meshUi.togglePkzSort}
              pkzSelectedIndices={meshUi.pkzSelectedIndices}
              setPkzSelectedIndices={meshUi.setPkzSelectedIndices}
              pkzAnchorIndex={meshUi.pkzAnchorIndex}
              setPkzAnchorIndex={meshUi.setPkzAnchorIndex}
              runStructMutation={actions.runStructMutation}
              runClothMutation={actions.runClothMutation}
              runContainerMutation={actions.runContainerMutation}
              openPkzCompressionModal={actions.openPkzCompressionModal}
              selectAndInspectNode={actions.selectAndInspectNode}
              handlePickOodleDll={dll.handlePickOodleDll}
              loadDirectory={actions.loadDirectory}
              currentDirRef={actions.currentDirRef}
              buildAssetNodeContextMenu={actions.buildAssetNodeContextMenu}
              setContextMenu={actions.setContextMenu}
              setHoverSubmenuIdx={actions.setHoverSubmenuIdx}
              setStatusLeft={actions.setStatusLeft}
            />
          ) : (
            <div
              style={{
                padding: 24,
                color: "var(--brush-muted)",
                textAlign: "center",
              }}
            >
              Select an asset node to edit.
            </div>
          )}
        </div>
      </div>

      <div
        className={`splitter-v ${draggingSplit === "right" ? "dragging" : ""}`}
        onMouseDown={(e) => {
          e.preventDefault();
          setDraggingSplit("right");
        }}
        onDoubleClick={() => setRightWidth(380)}
        data-tooltip="Drag to resize, double-click to reset"
      >
        <div className="splitter-v-grip" />
      </div>

      <div className="right-column" style={{ width: rightWidth }}>
        {activeTexture ? (
          <TextureInspectorView
            activeTexture={activeTexture}
            openCards={openCards}
            toggleCard={toggleCard}
            hashDraft={hashDraft}
            setHashDraft={setHashDraft}
            flagsDraft={flagsDraft}
            setFlagsDraft={setFlagsDraft}
            texEncodeFormat={texEncodeFormat}
            setTexEncodeFormat={setTexEncodeFormat}
            texQuality={texQuality}
            setTexQuality={setTexQuality}
            texGenerateMips={texGenerateMips}
            setTexGenerateMips={setTexGenerateMips}
            texSolidify={texSolidify}
            setTexSolidify={setTexSolidify}
            texMakeOpaque={texMakeOpaque}
            setTexMakeOpaque={setTexMakeOpaque}
            texSyncFlags={texSyncFlags}
            setTexSyncFlags={setTexSyncFlags}
            runTextureMutation={actions.runTextureMutation}
            promptAndRunTextureImport={actions.promptAndRunTextureImport}
            handleTextureExport={actions.handleTextureExport}
            uvdContext={nodePayload?.uvd ?? null}
            uvdActions={actions.uvdActions}
          />
        ) : nodePayload?.view_type === "UVD" && nodePayload.uvd ? (
          <div>
            <div className="inspector-header">
              <div className="inspector-icon-box">
                <IconImage size={20} />
              </div>
              <div>
                <div className="inspector-title">{nodePayload.title}</div>
                <div className="inspector-subtitle">{nodePayload.spec_line}</div>
              </div>
            </div>
            <UvdInspectorSection
              uvd={nodePayload.uvd}
              openCards={openCards}
              toggleCard={toggleCard}
              uvdActions={actions.uvdActions}
            />
          </div>
        ) : activeWmb ? (
          <WmbInspectorView
            activeWmb={activeWmb}
            openCards={openCards}
            toggleCard={toggleCard}
            wmbB2Override={wmbB2Override}
            wmbMatDraft={wmbMatDraft}
            setWmbMatDraft={setWmbMatDraft}
            runWmbMutation={actions.runWmbMutation}
          />
        ) : nodePayload?.view_type === "BXM" ? (
          <BxmInspectorView
            nodePayload={nodePayload}
            bxmTreeDraft={bxmTreeDraft}
            selectedBxmPath={selectedBxmPath}
            openCards={openCards}
            toggleCard={toggleCard}
            updateBxmDraftTree={updateBxmDraftTree}
            commitBxmDraft={actions.commitBxmDraft}
          />
        ) : nodePayload?.view_type === "CLOTH" ? (
          <ClothInspectorView
            nodePayload={nodePayload}
            setNodePayload={setNodePayload}
            openCards={openCards}
            toggleCard={toggleCard}
            runClothMutation={actions.runClothMutation}
          />
        ) : (
          <div>
            <div className="inspector-header">
              <div className="inspector-icon-box">
                <IconCube size={20} />
              </div>
              <div>
                <div className="inspector-title">
                  {nodePayload?.title ?? "No asset selected"}
                </div>
                <div className="inspector-subtitle">
                  {nodePayload?.spec_line ??
                    (outline
                      ? `${outline.root_format} · ${outline.endian}`
                      : "Open a container from the File Browser and pick an item from the Asset Browser.")}
                </div>
              </div>
            </div>

          </div>
        )}
      </div>
    </>
  );
}
