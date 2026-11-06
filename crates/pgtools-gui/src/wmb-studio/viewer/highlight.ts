// [wmb-studio BEGIN phase2]
import * as THREE from "three";
import { WmbSceneManifest } from "../contract";
import {
  findBoneBySceneId,
  findLodById,
  findMaterialById,
  findMeshGroupById,
  SceneSelectionItem,
} from "../store";

export interface SelectionHighlightSets {
  batchIds: Set<string>;
  boneIds: Set<number>;
}

/**
 * Computes which batches and bones should be highlighted for the active selection:
 * - Selecting a batch highlights that batch.
 * - Selecting a material highlights all batches using that material.
 * - Selecting a mesh group highlights all batches in that group.
 * - Selecting a LOD highlights all batches in that LOD.
 * - Selecting a texture highlights all batches whose material uses that texture.
 * - Selecting a bone highlights that bone in the skeleton overlay.
 */
export function computeHighlightSets(
  manifest: WmbSceneManifest | null,
  selection: SceneSelectionItem[]
): SelectionHighlightSets {
  const batchIds = new Set<string>();
  const boneIds = new Set<number>();
  if (!manifest) return { batchIds, boneIds };

  for (const item of selection) {
    if (item.kind === "batch") {
      batchIds.add(item.id);
    } else if (item.kind === "material") {
      const mat = findMaterialById(manifest, item.id);
      mat?.usedByBatchIds.forEach((bid) => batchIds.add(bid));
    } else if (item.kind === "meshGroup") {
      const mg = findMeshGroupById(manifest, item.id);
      if (mg) {
        mg.batchIds.forEach((bid) => batchIds.add(bid));
        mg.lods.forEach((l) => l.batchIds.forEach((bid) => batchIds.add(bid)));
      }
    } else if (item.kind === "lod") {
      const found = findLodById(manifest, item.id);
      found?.lod.batchIds.forEach((bid) => batchIds.add(bid));
    } else if (item.kind === "texture") {
      const raw = item.id.replace(/^tex:/, "");
      const tid = raw.toLowerCase().startsWith("0x")
        ? parseInt(raw.slice(2), 16)
        : parseInt(raw, 10);
      if (!Number.isNaN(tid)) {
        const texRef = manifest.textures.find(
          (t) => (t.textureId >>> 0) === (tid >>> 0)
        );
        texRef?.usedByMaterialIds.forEach((mid) => {
          const mat = findMaterialById(manifest, mid);
          mat?.usedByBatchIds.forEach((bid) => batchIds.add(bid));
        });
      }
    } else if (item.kind === "bone") {
      const bone = findBoneBySceneId(manifest, item.id);
      if (bone) {
        boneIds.add(bone.globalId);
      }
    }
  }

  return { batchIds, boneIds };
}

export function applyBatchHighlightUniforms(
  batchMeshes: Map<string, THREE.Mesh>,
  highlightedBatchIds: Set<string>
): void {
  const highlightedMats = new Set<THREE.ShaderMaterial>();
  batchMeshes.forEach((mesh, batchId) => {
    const isHighlighted = highlightedBatchIds.has(batchId);
    if (isHighlighted && mesh.material instanceof THREE.ShaderMaterial) {
      highlightedMats.add(mesh.material);
    }
    mesh.onBeforeRender = (_renderer, _scene, _camera, _geom, material) => {
      if (
        material instanceof THREE.ShaderMaterial &&
        material.uniforms?.uSelectionHighlight
      ) {
        const nextVal = isHighlighted ? 1.0 : 0.0;
        if (material.uniforms.uSelectionHighlight.value !== nextVal) {
          material.uniforms.uSelectionHighlight.value = nextVal;
          material.uniformsNeedUpdate = true;
        }
      }
    };
  });

  batchMeshes.forEach((mesh) => {
    const mat = mesh.material;
    if (mat instanceof THREE.ShaderMaterial && mat.uniforms?.uSelectionHighlight) {
      mat.uniforms.uSelectionHighlight.value = highlightedMats.has(mat)
        ? 1.0
        : 0.0;
    }
  });
}
// [wmb-studio END]
