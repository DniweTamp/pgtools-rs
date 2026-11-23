import * as THREE from "three";
import { WmbSceneManifest } from "../contract";
import {
  collectBatchIdsForSelection,
  findBoneBySceneId,
  SceneSelectionItem,
} from "../store";

interface SelectionHighlightSets {
  batchIds: Set<string>;
  boneIds: Set<number>;
}

export function computeHighlightSets(
  manifest: WmbSceneManifest | null,
  selection: SceneSelectionItem[]
): SelectionHighlightSets {
  const batchIds = collectBatchIdsForSelection(manifest, selection, true);
  const boneIds = new Set<number>();
  if (!manifest) return { batchIds, boneIds };

  for (const item of selection) {
    if (item.kind === "bone") {
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
  batchMeshes.forEach((mesh, batchId) => {
    const isHighlighted = highlightedBatchIds.has(batchId);
    const mat = mesh.material;
    if (mat instanceof THREE.ShaderMaterial && mat.uniforms?.uSelectionHighlight) {
      mat.uniforms.uSelectionHighlight.value = 0.0;
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
}
