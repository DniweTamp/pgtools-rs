import { invoke } from "@tauri-apps/api/core";
import { studioConfirm, studioPrompt } from "../components/StudioDialog";
import {
  DecodedWmbMaterial,
  FsEntry,
  MaterialClipboardState,
  MenuItemSpec,
  Wmb3Material,
  TextureNodeView,
  WmbSectionView,
} from "../types";
import { isVirtualSubNode } from "../utils/format";
import { convertMaterialBetweenGames } from "../wmb-studio/contract";
import { UvdActions } from "../editors/UvdEditor";

export interface AssetMenuDeps {
  selectedNodeIds?: string[];
  uvdActions?: UvdActions;
  activeTexture?: TextureNodeView | null;
  activeWmb: WmbSectionView | null;
  wmbMatDraft: DecodedWmbMaterial | null;
  wmb3MatDraft: Wmb3Material | null;
  wmbB2Override: boolean | null;
  materialClipboard: MaterialClipboardState | null;
  setMaterialClipboard: (val: MaterialClipboardState | null) => void;
  setContextMenu: (val: any) => void;
  setStatusLeft: (msg: string) => void;
  runWmbMutation: (nodeId: string, operation: string, payload?: any) => Promise<void>;
  handleCrossGameMaterialConvert: (wmb: WmbSectionView, dstGame: "b1" | "b2" | "b3") => Promise<void>;
  runContainerMutation: (nodeId: string, action: string, targetPath?: string, payload?: any) => Promise<void>;
  promptAndRunTextureImport: (nodeId: string, action: "replace_file" | "add_texture") => Promise<void>;
  handleTextureExport: (nodeId: string, scope: "single" | "bank" | "container", format: "png" | "dds" | "raw") => Promise<void>;
  openBatchReplaceModal?: (nodeId: string) => void;
  runTextureMutation: (nodeId: string, action: string, payload?: any) => Promise<void>;
  runClothMutation: (nodeId: string, operation: string, preset?: string, updatedJson?: any) => Promise<void>;
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
  onExpandAll: () => void;
  onCollapseAll: () => void;
  loadDirectory: (target?: string) => Promise<void>;
  currentDirRef: React.MutableRefObject<string>;
}

export interface FileMenuDeps {
  selectedFilePaths: string[];
  setSelectedFilePaths: (paths: string[]) => void;
  filteredFiles: FsEntry[];
  fsClipboard: { mode: "copy" | "cut"; paths: string[] } | null;
  setFsClipboard: (val: { mode: "copy" | "cut"; paths: string[] } | null) => void;
  currentDir: string;
  setContextMenu: (val: any) => void;
  setStatusLeft: (msg: string) => void;
  loadDirectory: (target?: string) => Promise<void>;
  openDocumentByPath: (path: string) => Promise<void>;
  onPackFolderToPkz: (folderPath: string) => Promise<void>;
}

export function createMenuBuilders(assetDeps: AssetMenuDeps, fileDeps: FileMenuDeps) {
  const {
    selectedNodeIds,
    uvdActions,
    activeTexture,
    activeWmb,
    wmbMatDraft,
    wmb3MatDraft,
    wmbB2Override,
    materialClipboard,
    setMaterialClipboard,
    setContextMenu,
    setStatusLeft,
    runWmbMutation,
    handleCrossGameMaterialConvert,
    runContainerMutation,
    promptAndRunTextureImport,
    handleTextureExport,
    openBatchReplaceModal,
    runTextureMutation,
    runClothMutation,
    openPkzCompressionModal,
    onExpandAll,
    onCollapseAll,
    loadDirectory,
    currentDirRef,
  } = assetDeps;

  const {
    selectedFilePaths,
    setSelectedFilePaths,
    filteredFiles,
    fsClipboard,
    setFsClipboard,
    currentDir,
    openDocumentByPath,
    onPackFolderToPkz,
  } = fileDeps;

  function buildAssetNodeContextMenu(nodeId: string, kind: string): MenuItemSpec[] {
    const virtualSub = isVirtualSubNode(nodeId);
    const leafSeg = nodeId.split("/").pop() || "";
    const isPkzEntry = !virtualSub && leafSeg.startsWith("pkz:");
    const isContainerEntry =
      !virtualSub &&
      nodeId !== "root" &&
      nodeId !== "root:dat" &&
      nodeId !== "root:dtt" &&
      (leafSeg.startsWith("dat:") || isPkzEntry || leafSeg.startsWith("efffile:"));
    const isContainer =
      kind === "DAT" ||
      kind === "DTT" ||
      kind === "PKZ" ||
      kind === "EFF" ||
      kind === "EFF_DIR" ||
      kind === "SCR";

    // 1. Virtual Material Sub-Node (`wmb:mat:X` or `wmb3:mat:X`)
    if (virtualSub && (leafSeg.startsWith("wmb:mat:") || leafSeg.startsWith("wmb3:mat:"))) {
      const isWmb3Mat = leafSeg.startsWith("wmb3:mat:");
      return [
        {
          label: "Copy Material to Clipboard",
          action: () => {
            setContextMenu(null);
            if (isWmb3Mat && wmb3MatDraft) {
              setMaterialClipboard({ game: "b3", payload: structuredClone(wmb3MatDraft) });
              setStatusLeft(`Copied WMB3 material '${wmb3MatDraft.name}' to clipboard`);
            } else if (wmbMatDraft) {
              const g = (wmbB2Override ?? activeWmb?.is_bayo2) ? "b2" : "b1";
              setMaterialClipboard({ game: g, payload: structuredClone(wmbMatDraft) });
              setStatusLeft(`Copied Material #${wmbMatDraft.index} to clipboard`);
            }
          },
        },
        {
          label: "Paste Material from Clipboard",
          disabled: !materialClipboard,
          action: async () => {
            setContextMenu(null);
            if (!materialClipboard || !activeWmb) return;
            const dstGame = activeWmb.is_wmb3
              ? "b3"
              : (wmbB2Override ?? activeWmb.is_bayo2)
              ? "b2"
              : "b1";
            if (materialClipboard.game === dstGame) {
              if (activeWmb.is_wmb3) {
                const matIdx = parseInt(activeWmb.section.replace("mat:", ""), 10) || 0;
                await runWmbMutation(activeWmb.node_id, "update_wmb3_material", {
                  index: matIdx,
                  material: materialClipboard.payload,
                });
              } else {
                const nextMat = {
                  ...materialClipboard.payload,
                  index: wmbMatDraft?.index ?? 0,
                };
                await runWmbMutation(activeWmb.node_id, "update_material", nextMat);
              }
            } else {
              const res = await convertMaterialBetweenGames(
                materialClipboard.game,
                dstGame,
                materialClipboard.payload,
                null
              );
              if (activeWmb.is_wmb3) {
                const matIdx = parseInt(activeWmb.section.replace("mat:", ""), 10) || 0;
                await runWmbMutation(activeWmb.node_id, "update_wmb3_material", {
                  index: matIdx,
                  material: res.converted,
                });
              } else {
                const nextMat = {
                  ...res.converted,
                  index: wmbMatDraft?.index ?? 0,
                };
                await runWmbMutation(activeWmb.node_id, "update_material", nextMat);
              }
            }
          },
        },
        { separator: true },
        {
          label: "Cross-Game Convert Material",
          submenu: [
            {
              label: "Convert → Bayonetta 1 (0x24..0xBC)",
              action: () => activeWmb && handleCrossGameMaterialConvert(activeWmb, "b1"),
            },
            {
              label: "Convert → Bayonetta 2 (843 Shaders)",
              action: () => activeWmb && handleCrossGameMaterialConvert(activeWmb, "b2"),
            },
            {
              label: "Convert → Bayonetta 3 (WMB3 PBS00)",
              action: () => activeWmb && handleCrossGameMaterialConvert(activeWmb, "b3"),
            },
          ],
        },
      ];
    }

    // 2. Other virtual WMB sub-nodes (`wmb:header`, `wmb:bones`, `wmb:meshes`, `wmb:vertexes`)
    if (virtualSub && (leafSeg.startsWith("wmb:") || leafSeg.startsWith("wmb3:"))) {
      const isWmb3Node = leafSeg.startsWith("wmb3:");
      return [
        {
          label: "Export Model to Wavefront .OBJ...",
          action: async () => {
            setContextMenu(null);
            const dest = await invoke<string | null>("save_file_dialog", {
              title: "Export WMB Model to .OBJ",
              defaultName: "model.obj",
            });
            if (dest) await runWmbMutation(nodeId, "export_obj", { path: dest });
          },
        },
        ...(isWmb3Node
          ? [
              {
                label: "Bake Skeleton & Vertices to T-Pose",
                action: () => runWmbMutation(nodeId, "set_tpose"),
              },
            ]
          : [
              {
                label: "Cleanup Unreferenced Vertexes",
                action: () => runWmbMutation(nodeId, "cleanup_vertexes"),
              },
              {
                label: "Cleanup Unused Materials",
                action: () => runWmbMutation(nodeId, "cleanup_materials"),
              },
              {
                label: "Recompute Relative Bone Positions",
                action: () => runWmbMutation(nodeId, "recompute_relative_positions"),
              },
              {
                label: "Swap Endianness (Wii U ↔ PC)",
                action: () => runWmbMutation(nodeId, "swap_endian"),
              },
            ]),
      ];
    }

    // Standard leaf entry actions inside a parent container
    const leafEntryMgmtItems: MenuItemSpec[] = isContainerEntry
      ? [
          {
            label: "Replace Sub-File from Disk...",
            action: async () => {
              setContextMenu(null);
              const src = await invoke<string | null>("pick_file_dialog", {
                title: "Select Replacement Sub-File",
                extensions: null,
              });
              if (!src) return;
              if (isPkzEntry) {
                openPkzCompressionModal(
                  "Replace PKZ Entry",
                  `Choose compression for replacement file '${src.split(/[\\/]/).pop()}'`,
                  undefined,
                  async (compression, level, forceCompression) => {
                    await runContainerMutation(nodeId, "replace_subfile", src, {
                      compression,
                      level,
                      force_compression: forceCompression,
                    });
                  }
                );
              } else {
                await runContainerMutation(nodeId, "replace_subfile", src);
              }
            },
          },
          ...(isPkzEntry
            ? [
                {
                  label: "Recompress Entry...",
                  action: () => {
                    setContextMenu(null);
                    openPkzCompressionModal(
                      "Recompress PKZ Entry",
                      "Select compression codec and level for this entry",
                      undefined,
                      async (compression, level, forceCompression) => {
                        await runContainerMutation(nodeId, "set_pkz_compression", compression, {
                          compression,
                          level,
                          force_compression: forceCompression,
                        });
                      }
                    );
                  },
                },
              ]
            : []),
          {
            label: "Export Raw Sub-File...",
            action: async () => {
              setContextMenu(null);
              const dest = await invoke<string | null>("save_file_dialog", {
                title: "Export Raw Sub-File",
                defaultName: "extracted.bin",
              });
              if (dest) {
                await runContainerMutation(nodeId, "export_subfile", dest);
                setStatusLeft(`Exported raw sub-file to ${dest}`);
              }
            },
          },
          {
            label: "Rename Entry...",
            shortcut: "F2",
            action: async () => {
              setContextMenu(null);
              const nextName = await studioPrompt("Enter new entry filename:", "", { title: "Rename Entry" });
              if (nextName && nextName.trim()) {
                await runContainerMutation(nodeId, "rename_entry", nextName.trim(), {
                  new_name: nextName.trim(),
                });
              }
            },
          },
          ...(isPkzEntry
            ? [
                {
                  label: "Revert This Row",
                  action: () => {
                    setContextMenu(null);
                    void runContainerMutation(nodeId, "revert_pkz_entries");
                  },
                },
              ]
            : []),
          {
            label: "Delete from Container",
            shortcut: "Del",
            action: async () => {
              setContextMenu(null);
              if (isPkzEntry) {
                const ok = await studioConfirm(
                  `Delete '${nodeId}' from the PKZ archive?\n\nThe change is staged until you save.`,
                  { title: "Delete Entry", confirmLabel: "Delete", danger: true }
                );
                if (!ok) return;
              }
              void runContainerMutation(nodeId, "remove_entry");
            },
          },
        ]
      : [];

    // 2B. PKZ Virtual Folder Node (`PKZ_FOLDER` / `pkzfolder:<prefix>`)
    if (kind === "PKZ_FOLDER" || nodeId.includes("pkzfolder:")) {
      const folderPrefix = nodeId.includes("pkzfolder:")
        ? nodeId.split("pkzfolder:").pop() || ""
        : leafSeg.replace(/^pkzfolder:/, "");
      return [
        {
          label: `Add File(s) into '${folderPrefix}/'...`,
          action: async () => {
            setContextMenu(null);
            const paths = await invoke<string[]>("pick_multiple_files_dialog", {
              title: `Add File(s) into PKZ Folder '${folderPrefix}/'`,
              extensions: null,
            });
            if (paths && paths.length > 0) {
              openPkzCompressionModal(
                `Add ${paths.length} File(s) to '${folderPrefix}/'`,
                "Choose compression codec and level for added files",
                undefined,
                async (compression, level, forceCompression) => {
                  await runContainerMutation(nodeId, "add_files", "", {
                    paths,
                    folder_prefix: folderPrefix,
                    compression,
                    level,
                    force_compression: forceCompression,
                  });
                }
              );
            }
          },
        },
        {
          label: `Add Folder into '${folderPrefix}/'...`,
          action: async () => {
            setContextMenu(null);
            const folder = await invoke<string | null>("pick_folder_dialog", {
              title: `Select Folder whose contents will be added into '${folderPrefix}/'`,
            });
            if (folder) {
              openPkzCompressionModal(
                `Add Folder into '${folderPrefix}/'`,
                "Choose compression codec and level for added folder contents",
                undefined,
                async (compression, level, forceCompression) => {
                  await runContainerMutation(nodeId, "add_files", "", {
                    paths: [folder],
                    folder_prefix: folderPrefix,
                    compression,
                    level,
                    force_compression: forceCompression,
                  });
                }
              );
            }
          },
        },
        {
          label: `Extract Folder '${folderPrefix}/' to Disk...`,
          action: async () => {
            setContextMenu(null);
            const folder = await invoke<string | null>("pick_folder_dialog", {
              title: `Extract PKZ Folder '${folderPrefix}'`,
            });
            if (folder) {
              await runContainerMutation(nodeId, "extract_pkz_folder", folder, {
                folder_prefix: folderPrefix,
              });
              setStatusLeft(`Extracted PKZ folder '${folderPrefix}' to ${folder}`);
              loadDirectory(currentDirRef.current);
            }
          },
        },
        {
          label: `Recompress Folder '${folderPrefix}/'...`,
          action: () => {
            setContextMenu(null);
            openPkzCompressionModal(
              `Recompress Folder '${folderPrefix}/'`,
              "Change compression codec and level for all files under this folder",
              undefined,
              async (compression, level, forceCompression) => {
                await runContainerMutation(nodeId, "set_pkz_compression", compression, {
                  folder_prefix: folderPrefix,
                  compression,
                  level,
                  force_compression: forceCompression,
                });
              }
            );
          },
        },
        {
          label: "Rename Folder...",
          shortcut: "F2",
          action: async () => {
            setContextMenu(null);
            const nextPrefix = await studioPrompt("Enter new PKZ folder path:", folderPrefix, { title: "Rename PKZ Folder" });
            if (nextPrefix !== null && nextPrefix.trim() !== folderPrefix) {
              await runContainerMutation(nodeId, "rename_pkz_folder", nextPrefix.trim(), {
                folder_prefix: folderPrefix,
                new_prefix: nextPrefix.trim(),
              });
            }
          },
        },
        {
          label: `Revert Folder '${folderPrefix}/'`,
          action: () => {
            setContextMenu(null);
            void runContainerMutation(nodeId, "revert_pkz_entries", "", {
              folder_prefix: folderPrefix,
            });
          },
        },
        { separator: true },
        {
          label: `Delete Folder '${folderPrefix}/' from PKZ`,
          shortcut: "Del",
          action: async () => {
            setContextMenu(null);
            const ok = await studioConfirm(
              `Delete the folder '${folderPrefix}' and all files under it?\n\nThe change is staged until you save.`,
              { title: "Delete Folder", confirmLabel: "Delete", danger: true }
            );
            if (!ok) return;
            void runContainerMutation(nodeId, "remove_pkz_folder", "", {
              folder_prefix: folderPrefix,
            });
          },
        },
        { separator: true },
        {
          label: "Expand All",
          action: () => {
            setContextMenu(null);
            onExpandAll();
          },
        },
        {
          label: "Collapse All",
          action: () => {
            setContextMenu(null);
            onCollapseAll();
          },
        },
      ];
    }

    // 3. Container Node (`DAT`, `DTT`, `PKZ`, `EFF`, `EFF_DIR`, `SCR`)
    if (isContainer) {
      const items: MenuItemSpec[] = [];
      if (kind === "PKZ") {
        items.push(
          {
            label: "Add File(s)...",
            action: async () => {
              setContextMenu(null);
              const paths = await invoke<string[]>("pick_multiple_files_dialog", {
                title: "Select File(s) to Add to PKZ",
                extensions: null,
              });
              if (paths && paths.length > 0) {
                openPkzCompressionModal(
                  "Add File(s) as",
                  "Choose compression codec and level for added files",
                  undefined,
                  async (compression, level, forceCompression) => {
                    await runContainerMutation(nodeId, "add_files", "", {
                      paths,
                      compression,
                      level,
                      force_compression: forceCompression,
                    });
                  }
                );
              }
            },
          },
          {
            label: "Add Folder...",
            action: async () => {
              setContextMenu(null);
              const folder = await invoke<string | null>("pick_folder_dialog", {
                title: "Select Folder to Add into PKZ",
              });
              if (folder) {
                openPkzCompressionModal(
                  "Add Folder Contents as",
                  "Choose compression codec and level for added folder contents",
                  undefined,
                  async (compression, level, forceCompression) => {
                    await runContainerMutation(nodeId, "add_files", "", {
                      paths: [folder],
                      compression,
                      level,
                      force_compression: forceCompression,
                    });
                  }
                );
              }
            },
          },
          {
            label: "Recompress All...",
            action: () => {
              setContextMenu(null);
              openPkzCompressionModal(
                "Recompress All PKZ Entries",
                "Choose compression codec and level for all entries in this archive",
                undefined,
                async (compression, level, forceCompression) => {
                  await runContainerMutation(nodeId, "set_pkz_compression", compression, {
                    compression,
                    level,
                    force_compression: forceCompression,
                  });
                }
              );
            },
          },
          {
            label: "Revert All Staged Changes",
            action: () => {
              setContextMenu(null);
              void runContainerMutation(nodeId, "revert_pkz_entries");
            },
          },
          { separator: true }
        );
      } else if (kind !== "SCR") {
        items.push({
          label: "Add File(s) to Container...",
          action: async () => {
            setContextMenu(null);
            const paths = await invoke<string[]>("pick_multiple_files_dialog", {
              title: "Select File(s) to Add to Container",
              extensions: null,
            });
            if (paths && paths.length > 0) {
              await runContainerMutation(nodeId, "add_files", "", { paths });
            }
          },
        });
      }
      items.push(
        {
          label: "Extract All to Folder...",
          action: async () => {
            setContextMenu(null);
            const folder = await invoke<string | null>("pick_folder_dialog", {
              title: "Extract Container Entries to Folder",
            });
            if (folder) {
              await runContainerMutation(nodeId, "extract_all", folder);
              setStatusLeft(`Extracted all entries to ${folder}`);
              loadDirectory(currentDirRef.current);
            }
          },
        },
        {
          label: "Export every texture in this container",
          submenu: [
            {
              label: "As PNG folder...",
              action: () => handleTextureExport(nodeId, "container", "png"),
            },
            {
              label: "As DDS folder...",
              action: () => handleTextureExport(nodeId, "container", "dds"),
            },
            {
              label: "As raw folder...",
              action: () => handleTextureExport(nodeId, "container", "raw"),
            },
          ],
        }
      );
      if (nodeId !== "root") {
        items.push({
          label: "Export Raw Container...",
          action: async () => {
            setContextMenu(null);
            const dest = await invoke<string | null>("save_file_dialog", {
              title: "Export Raw Container",
              defaultName: `archive.${kind.toLowerCase()}`,
            });
            if (dest) {
              await runContainerMutation(nodeId, "export_subfile", dest);
              setStatusLeft(`Exported container to ${dest}`);
            }
          },
        });
      }
      if (leafEntryMgmtItems.length > 0) {
        items.push({ separator: true }, ...leafEntryMgmtItems);
      }
      items.push(
        { separator: true },
        {
          label: "Expand All",
          action: () => {
            setContextMenu(null);
            onExpandAll();
          },
        },
        {
          label: "Collapse All",
          action: () => {
            setContextMenu(null);
            onCollapseAll();
          },
        }
      );
      return items;
    }

    // 3B. UVD / UVD_GROUP / UVD_ITEM Context Menus
    if ((kind === "UVD" || kind === "UVD_GROUP" || kind === "UVD_ITEM") && uvdActions) {
      const multiIds =
        selectedNodeIds && selectedNodeIds.length > 1 && selectedNodeIds.includes(nodeId)
          ? selectedNodeIds
          : [nodeId];

      if (kind === "UVD") {
        return [
          {
            label: "Extract All Groups (PNG Subfolders)...",
            action: () => {
              setContextMenu(null);
              void uvdActions.handleUvdExtract([nodeId], false);
            },
          },
          {
            label: "Extract All Groups + .uvd.json...",
            action: () => {
              setContextMenu(null);
              void uvdActions.handleUvdExtract([nodeId], true);
            },
          },
          { separator: true },
          {
            label: "Add New Group...",
            action: () => {
              setContextMenu(null);
              void uvdActions.promptAddUvdGroup(nodeId);
            },
          },
          {
            label: "Swap Endianness (Little ↔ Big)",
            action: () => {
              setContextMenu(null);
              void uvdActions.runUvdMutation(nodeId, "swap_endian");
            },
          },
          ...(leafEntryMgmtItems.length > 0 ? [{ separator: true }, ...leafEntryMgmtItems] : []),
          { separator: true },
          {
            label: "Expand All",
            action: () => {
              setContextMenu(null);
              onExpandAll();
            },
          },
          {
            label: "Collapse All",
            action: () => {
              setContextMenu(null);
              onCollapseAll();
            },
          },
        ];
      }

      if (kind === "UVD_GROUP") {
        const groupIndices = multiIds
          .map((id) => {
            const m = id.match(/uvdgrp:(\d+)$/);
            return m ? parseInt(m[1], 10) : -1;
          })
          .filter((n) => n >= 0);
        const countLabel = groupIndices.length > 1 ? `${groupIndices.length} Selected Groups` : "Group";

        return [
          {
            label: `Extract ${countLabel} (PNG Subfolder)...`,
            action: () => {
              setContextMenu(null);
              void uvdActions.handleUvdExtract(multiIds, false);
            },
          },
          {
            label: `Extract ${countLabel} + .uvd.json...`,
            action: () => {
              setContextMenu(null);
              void uvdActions.handleUvdExtract(multiIds, true);
            },
          },
          { separator: true },
          {
            label: "Add Sprite to Group...",
            disabled: groupIndices.length > 1,
            action: () => {
              setContextMenu(null);
              void uvdActions.promptAddUvdSprite(nodeId, groupIndices[0]);
            },
          },
          {
            label: "Repack Group (Apply Padding)",
            disabled: groupIndices.length > 1,
            action: () => {
              setContextMenu(null);
              void uvdActions.runUvdMutation(nodeId, "repack_group", {
                group_index: groupIndices[0],
              });
            },
          },
          { separator: true },
          {
            label: "Rename Group...",
            shortcut: "F2",
            disabled: groupIndices.length > 1,
            action: () => {
              setContextMenu(null);
              const fallback = activeTexture?.embedded_name || `group_${groupIndices[0] ?? 0}`;
              const curName = uvdActions.resolveNodeName
                ? uvdActions.resolveNodeName(nodeId, fallback)
                : fallback;
              void uvdActions.promptRenameUvdGroup(nodeId, curName, groupIndices[0]);
            },
          },
          {
            label: `Move ${countLabel} Up`,
            action: () => {
              setContextMenu(null);
              void uvdActions.runUvdMutation(nodeId, "reorder_groups", {
                group_indices: groupIndices,
                direction: "up",
              });
            },
          },
          {
            label: `Move ${countLabel} Down`,
            action: () => {
              setContextMenu(null);
              void uvdActions.runUvdMutation(nodeId, "reorder_groups", {
                group_indices: groupIndices,
                direction: "down",
              });
            },
          },
          { separator: true },
          {
            label: `Delete ${countLabel}`,
            shortcut: "Del",
            action: async () => {
              setContextMenu(null);
              const ok = await studioConfirm(
                `Delete ${groupIndices.length} UVD group(s)?\n\nWTA texture slots will not be deleted or reordered.`,
                { title: "Delete UVD Groups", confirmLabel: "Delete", danger: true }
              );
              if (ok) {
                void uvdActions.runUvdMutation(nodeId, "delete_groups", {
                  group_indices: groupIndices,
                });
              }
            },
          },
        ];
      }

      if (kind === "UVD_ITEM") {
        const parsedPairs: [number, number][] = multiIds
          .map((id) => {
            const m = id.match(/uvdgrp:(\d+)\/uvditem:(\d+)$/);
            return m ? ([parseInt(m[1], 10), parseInt(m[2], 10)] as [number, number]) : null;
          })
          .filter((x): x is [number, number] => x !== null);
        const firstPair = parsedPairs[0] ?? [0, 0];
        const countLabel = parsedPairs.length > 1 ? `${parsedPairs.length} Selected Sprites` : "Sprite";

        return [
          {
            label:
              parsedPairs.length > 1
                ? `Extract ${countLabel} to Folder...`
                : "Extract Single Sprite (.PNG)...",
            action: () => {
              setContextMenu(null);
              void uvdActions.handleUvdExtract(multiIds, false);
            },
          },
          {
            label: "Replace Sprite (Single PNG)...",
            disabled: parsedPairs.length > 1,
            action: () => {
              setContextMenu(null);
              void uvdActions.promptReplaceUvdSprite(nodeId, firstPair[0], firstPair[1]);
            },
          },
          { separator: true },
          {
            label: "Rename Sprite...",
            shortcut: "F2",
            disabled: parsedPairs.length > 1,
            action: () => {
              setContextMenu(null);
              const fallback = activeTexture?.embedded_name || `sprite_${firstPair[1]}`;
              const curName = uvdActions.resolveNodeName
                ? uvdActions.resolveNodeName(nodeId, fallback)
                : fallback;
              void uvdActions.promptRenameUvdSprite(nodeId, curName, firstPair[0], firstPair[1]);
            },
          },
          { separator: true },
          {
            label: `Delete ${countLabel}`,
            shortcut: "Del",
            action: async () => {
              setContextMenu(null);
              const ok = await studioConfirm(
                `Delete ${parsedPairs.length} sprite(s) and repack affected group(s)?`,
                { title: "Delete Sprites", confirmLabel: "Delete", danger: true }
              );
              if (ok) {
                void uvdActions.runUvdMutation(nodeId, "delete_items", {
                  items: parsedPairs,
                });
              }
            },
          },
        ];
      }
    }

    // 4. Texture / WTB / BNTX / DDS / GTX / XT1
    if (
      kind === "TEXTURE" ||
      kind === "WTB" ||
      kind === "DDS" ||
      kind === "GTX" ||
      kind === "XT1" ||
      kind === "BNTX"
    ) {
      const targetId =
        kind === "WTB" || kind === "BNTX"
          ? nodeId === "root"
            ? "tex:0"
            : `${nodeId}/tex:0`
          : nodeId;
      return [
        {
          label: "Replace Texture (PNG / DDS / GTX / XT1 / BNTX)...",
          action: () => promptAndRunTextureImport(targetId, "replace_file"),
        },
        {
          label: "Dummy out",
          submenu: [
            {
              label: "White (8x8)",
              action: () => {
                setContextMenu(null);
                runTextureMutation(targetId, "dummy_out", { kind: "white" });
              },
            },
            {
              label: "Black (8x8)",
              action: () => {
                setContextMenu(null);
                runTextureMutation(targetId, "dummy_out", { kind: "black" });
              },
            },
            {
              label: "Normal (128, 128, 255)",
              action: () => {
                setContextMenu(null);
                runTextureMutation(targetId, "dummy_out", { kind: "normal" });
              },
            },
          ],
        },
        { separator: true },
        {
          label: "Export",
          submenu: [
            {
              label: "Export .png...",
              action: () => handleTextureExport(targetId, "single", "png"),
            },
            {
              label: "Export .dds...",
              action: () => handleTextureExport(targetId, "single", "dds"),
            },
            {
              label: "Export raw payload...",
              action: () => handleTextureExport(targetId, "single", "raw"),
            },
          ],
        },
        { separator: true },
        {
          label: "Add texture to bank...",
          action: () => promptAndRunTextureImport(targetId, "add_texture"),
        },
        {
          label: "Delete texture from bank",
          action: () => {
            setContextMenu(null);
            runTextureMutation(targetId, "delete_texture");
          },
        },
        { separator: true },
        {
          label: "Copy hash",
          action: () => {
            setContextMenu(null);
            const h = activeTexture?.texture_id_hex || "0x00000000";
            navigator.clipboard?.writeText(h);
            setStatusLeft(`Copied texture hash ${h} to clipboard`);
          },
        },
        { separator: true },
        {
          label: "Export every texture in this bank",
          submenu: [
            {
              label: "As PNG folder...",
              action: () => handleTextureExport(targetId, "bank", "png"),
            },
            {
              label: "As DDS folder...",
              action: () => handleTextureExport(targetId, "bank", "dds"),
            },
            {
              label: "As raw folder...",
              action: () => handleTextureExport(targetId, "bank", "raw"),
            },
          ],
        },
        {
          label: "Export every texture in this container",
          submenu: [
            {
              label: "As PNG folder...",
              action: () => handleTextureExport(targetId, "container", "png"),
            },
            {
              label: "As DDS folder...",
              action: () => handleTextureExport(targetId, "container", "dds"),
            },
            {
              label: "As raw folder...",
              action: () => handleTextureExport(targetId, "container", "raw"),
            },
          ],
        },
        {
          label: "Convert Wii U (.gtx) bank to PC (.dds)",
          action: () => {
            setContextMenu(null);
            runTextureMutation(targetId, "convert_wiiu_to_pc");
          },
        },
        {
          label: "Batch replace from folder...",
          action: async () => {
            setContextMenu(null);
            if (openBatchReplaceModal) {
              openBatchReplaceModal(targetId);
            } else {
              const folder = await invoke<string | null>("pick_folder_dialog", {
                title: "Select Folder of Replacement Textures (matched by tex<idx> or hash)",
              });
              if (folder) await runTextureMutation(targetId, "batch_replace_folder", { folder });
            }
          },
        },
        ...(leafEntryMgmtItems.length > 0 ? [{ separator: true }, ...leafEntryMgmtItems] : []),
      ];
    }

    // 5. WMB / WMB3 Model File
    if (kind.startsWith("WMB")) {
      const isWmb3 = kind === "WMB3";
      return [
        {
          label: "Export Model to Wavefront .OBJ...",
          action: async () => {
            setContextMenu(null);
            const dest = await invoke<string | null>("save_file_dialog", {
              title: "Export WMB Model to .OBJ",
              defaultName: "model.obj",
            });
            if (dest) await runWmbMutation(nodeId, "export_obj", { path: dest });
          },
        },
        {
          label: "Check & Remap Texture Hashes (CRC32 & 7F)",
          action: () => runWmbMutation(nodeId, "remap_texture_hashes"),
        },
        ...(isWmb3
          ? [
              {
                label: "Bake WMB3 Skeleton & Vertices to T-Pose",
                action: () => runWmbMutation(nodeId, "set_tpose"),
              },
            ]
          : [
              { separator: true },
              {
                label: "Cleanup & Optimization",
                submenu: [
                  {
                    label: "Cleanup Unreferenced Vertexes",
                    action: () => runWmbMutation(nodeId, "cleanup_vertexes"),
                  },
                  {
                    label: "Cleanup Unused Materials",
                    action: () => runWmbMutation(nodeId, "cleanup_materials"),
                  },
                  {
                    label: "Clamp Material Sizes to Shader Spec",
                    action: () => runWmbMutation(nodeId, "cleanup_material_sizes"),
                  },
                  {
                    label: "Maximize Material Sizes",
                    action: () => runWmbMutation(nodeId, "maximize_material_sizes"),
                  },
                  {
                    label: "Convert Triangle Strips to Triangles",
                    action: () => runWmbMutation(nodeId, "remove_triangle_strips"),
                  },
                ],
              },
              {
                label: "Normals & Skeleton",
                submenu: [
                  {
                    label: "Convert Normals to 10-10-10-2 Wide",
                    action: () => runWmbMutation(nodeId, "normals_to_wide"),
                  },
                  {
                    label: "Check & Normalize Vertex Normals",
                    action: () => runWmbMutation(nodeId, "check_normals"),
                  },
                  {
                    label: "Recompute Relative Bone Positions",
                    action: () => runWmbMutation(nodeId, "recompute_relative_positions"),
                  },
                ],
              },
              {
                label: "Swap Endianness (Wii U ↔ PC)",
                action: () => runWmbMutation(nodeId, "swap_endian"),
              },
            ]),
        ...(leafEntryMgmtItems.length > 0 ? [{ separator: true }, ...leafEntryMgmtItems] : []),
      ];
    }

    // 6. Cloth Physics (`CLP`, `CLH`, `CLW`)
    if (kind === "CLP" || kind === "CLH" || kind === "CLW") {
      return [
        {
          label: "Remap Bone IDs (Cross-Game Presets)",
          submenu: [
            {
              label: "Bayonetta 2 (pl0010) → Bayonetta 1 (pl0010)",
              action: () => runClothMutation(nodeId, "remap_bones", "bayo2_pl0010_to_bayo1"),
            },
            {
              label: "Bayonetta 2 (pl0000) → Bayonetta 1 (pl0010)",
              action: () => runClothMutation(nodeId, "remap_bones", "bayo2_pl0000_to_bayo1"),
            },
            {
              label: "Bayonetta 2 Combat Head → Bayonetta 1",
              action: () => runClothMutation(nodeId, "remap_bones", "bayo2_combat_head"),
            },
            {
              label: "Bayonetta 2 Cutscene Head → Bayonetta 1",
              action: () => runClothMutation(nodeId, "remap_bones", "bayo2_cutscene_head"),
            },
            {
              label: "Bayonetta 2 Common Bones → Bayonetta 1",
              action: () => runClothMutation(nodeId, "remap_bones", "bayo2_common"),
            },
            {
              label: "NieR:Automata → Bayonetta 1",
              action: () => runClothMutation(nodeId, "remap_bones", "nier_to_bayo1"),
            },
          ],
        },
        {
          label: "Swap Endianness (Wii U Big ↔ PC Little)",
          action: () => runClothMutation(nodeId, "swap_endian"),
        },
        ...(leafEntryMgmtItems.length > 0 ? [{ separator: true }, ...leafEntryMgmtItems] : []),
      ];
    }

    // 7. BXM / XML / GAD
    if (kind === "BXM") {
      return [
        {
          label: "Export Decoded XML (.xml)...",
          action: async () => {
            setContextMenu(null);
            const dest = await invoke<string | null>("save_file_dialog", {
              title: "Export Decoded XML (.xml)",
              defaultName: "decoded.xml",
            });
            if (dest) {
              await runContainerMutation(nodeId, "export_bxm_xml", dest);
              setStatusLeft(`Exported decoded UTF-8 XML to ${dest}`);
              loadDirectory(currentDirRef.current);
            }
          },
        },
        {
          label: "Import & Encode from .XML...",
          action: async () => {
            setContextMenu(null);
            const srcXml = await invoke<string | null>("pick_file_dialog", {
              title: "Import & Encode XML into Binary BXM",
              extensions: ["xml"],
            });
            if (srcXml) {
              await runContainerMutation(nodeId, "import_bxm_xml", srcXml);
              setStatusLeft(`Imported and encoded ${srcXml} into BXM`);
            }
          },
        },
        ...(leafEntryMgmtItems.length > 0 ? [{ separator: true }, ...leafEntryMgmtItems] : []),
      ];
    }

    if (leafEntryMgmtItems.length > 0) {
      return leafEntryMgmtItems;
    }

    return [
      {
        label: "Export Raw File...",
        action: async () => {
          setContextMenu(null);
          const dest = await invoke<string | null>("save_file_dialog", {
            title: "Export Raw File",
            defaultName: "extracted.bin",
          });
          if (dest) await runContainerMutation(nodeId, "export_subfile", dest);
        },
      },
      {
        label: "Replace from Disk...",
        action: async () => {
          setContextMenu(null);
          const src = await invoke<string | null>("pick_file_dialog", {
            title: "Select Replacement File",
            extensions: null,
          });
          if (src) await runContainerMutation(nodeId, "replace_subfile", src);
        },
      },
    ];
  }

  function buildFileBrowserContextMenu(entry: FsEntry): MenuItemSpec[] {
    const ext = entry.extension.toLowerCase();
    const runCliOp = async (op: string, dest?: string) => {
      setContextMenu(null);
      try {
        setStatusLeft(`Running ${op} on ${entry.name}...`);
        const msg = await invoke<string>("run_file_context_operation", {
          filePath: entry.path,
          operation: op,
          destPath: dest ?? null,
        });
        setStatusLeft(msg);
        loadDirectory(currentDirRef.current);
      } catch (err) {
        setStatusLeft(`Operation failed: ${String(err)}`);
      }
    };

    const items: MenuItemSpec[] = [
      {
        label: entry.is_dir ? "Open Folder" : "Open in Studio",
        action: () => {
          setContextMenu(null);
          if (entry.is_dir) {
            loadDirectory(entry.path);
          } else {
            openDocumentByPath(entry.path);
          }
        },
      },
      {
        label: "Reload Directory",
        action: () => {
          setContextMenu(null);
          loadDirectory(currentDirRef.current);
        },
      },
      { separator: true },
    ];

    if (entry.is_dir) {
      items.push(
        {
          label: "Repack Folder into .DAT / .DTT Archive",
          action: () => runCliOp("repack_folder_dat"),
        },
        {
          label: "Pack Folder into .PKZ Archive...",
          action: () => {
            setContextMenu(null);
            void onPackFolderToPkz(entry.path);
          },
        },
        { separator: true }
      );
    } else {
      if (["dat", "dtt", "evn", "pkz", "eff", "idd", "scr", "wtb", "wta", "bntx"].includes(ext)) {
        items.push({
          label: `Extract .${ext.toUpperCase()} to Folder...`,
          action: () => runCliOp("extract_archive"),
        });
      }
      if (["wtb", "wta"].includes(ext)) {
        items.push({
          label: "Convert Wii U (.gtx) → PC (.wtb)",
          action: () => runCliOp("convert_wtb_wiiu_pc"),
        });
      }
      if (ext === "wmb" || ext === "mdb") {
        items.push({
          label: "Export Model to Wavefront .OBJ",
          action: () => runCliOp("wmb_export_obj"),
        });
        if (ext === "wmb") {
          items.push({
            label: "Swap Endianness (Wii U ↔ PC)",
            action: () => runCliOp("swap_endian_file"),
          });
        }
      }
      if (ext === "bxm" || ext === "gad") {
        items.push({
          label: "Decode Binary .BXM/.GAD → .XML",
          action: () => runCliOp("bxm_decode_xml"),
        });
      }
      if (ext === "xml") {
        items.push({
          label: "Encode .XML → Binary .BXM",
          action: () => runCliOp("bxm_encode_xml"),
        });
      }
      if (["clp", "clh", "clw", "mot", "exp", "uvd"].includes(ext)) {
        items.push({
          label: `Swap .${ext.toUpperCase()} Endianness (Wii U ↔ PC)`,
          action: () => runCliOp("swap_endian_file"),
        });
      }
      items.push({ separator: true });
    }

    const targetPaths =
      selectedFilePaths.includes(entry.path) && selectedFilePaths.length > 1
        ? selectedFilePaths
        : [entry.path];

    const runFsBatch = async (
      op: string,
      paths: string[],
      destDir?: string,
      newName?: string
    ) => {
      setContextMenu(null);
      try {
        const msg = await invoke<string>("fs_batch_operation", {
          operation: op,
          sourcePaths: paths,
          destDir: destDir ?? null,
          newName: newName ?? null,
        });
        setStatusLeft(msg);
        await loadDirectory(currentDirRef.current);
      } catch (err) {
        setStatusLeft(`File operation failed: ${String(err)}`);
      }
    };

    items.push(
      {
        label: targetPaths.length > 1 ? `Copy (${targetPaths.length} items)` : "Copy",
        shortcut: "Ctrl+C",
        action: () => {
          setContextMenu(null);
          setFsClipboard({ mode: "copy", paths: targetPaths });
          setStatusLeft(`Copied ${targetPaths.length} item(s) to clipboard`);
        },
      },
      {
        label: targetPaths.length > 1 ? `Cut (${targetPaths.length} items)` : "Cut",
        shortcut: "Ctrl+X",
        action: () => {
          setContextMenu(null);
          setFsClipboard({ mode: "cut", paths: targetPaths });
          setStatusLeft(`Cut ${targetPaths.length} item(s) to clipboard`);
        },
      },
      {
        label: fsClipboard
          ? `Paste into Current Folder (${fsClipboard.paths.length})`
          : "Paste",
        shortcut: "Ctrl+V",
        disabled: !fsClipboard || fsClipboard.paths.length === 0,
        action: async () => {
          if (!fsClipboard || !currentDir) return;
          const op = fsClipboard.mode === "cut" ? "paste_cut" : "paste_copy";
          const paths = fsClipboard.paths;
          if (fsClipboard.mode === "cut") setFsClipboard(null);
          await runFsBatch(op, paths, currentDir);
        },
      },
      {
        label: "Duplicate",
        action: () => runFsBatch("duplicate", targetPaths),
      },
      {
        label: "Rename...",
        shortcut: "F2",
        action: async () => {
          setContextMenu(null);
          const nextName = await studioPrompt("Rename to:", entry.name, { title: "Rename" });
          if (nextName && nextName.trim() && nextName.trim() !== entry.name) {
            await runFsBatch("rename", [entry.path], undefined, nextName.trim());
          }
        },
      },
      {
        label: targetPaths.length > 1 ? `Delete (${targetPaths.length} items)` : "Delete",
        shortcut: "Del",
        action: async () => {
          setContextMenu(null);
          const ok = await studioConfirm(
            targetPaths.length > 1
              ? `Delete ${targetPaths.length} selected items from disk?`
              : `Delete '${entry.name}' from disk?`,
            { title: "Delete from Disk", confirmLabel: "Delete", danger: true }
          );
          if (ok) {
            await runFsBatch("delete", targetPaths);
            setSelectedFilePaths([]);
          }
        },
      },
      { separator: true },
      {
        label: "New Folder...",
        action: async () => {
          setContextMenu(null);
          const folderName = await studioPrompt("New folder name:", "New Folder", { title: "New Folder" });
          if (folderName && folderName.trim()) {
            await runFsBatch("new_folder", [], currentDir, folderName.trim());
          }
        },
      },
      {
        label: "Select All",
        shortcut: "Ctrl+A",
        action: () => {
          setContextMenu(null);
          setSelectedFilePaths(filteredFiles.map((f) => f.path));
        },
      },
      { separator: true },
      {
        label: "Copy full path",
        action: () => {
          setContextMenu(null);
          navigator.clipboard?.writeText(entry.path);
          setStatusLeft(`Copied ${entry.path}`);
        },
      },
      {
        label: "Show in Explorer",
        action: () => {
          setContextMenu(null);
          invoke("reveal_in_explorer", { path: entry.path });
        },
      }
    );

    return items;
  }


  return { buildAssetNodeContextMenu, buildFileBrowserContextMenu };
}
