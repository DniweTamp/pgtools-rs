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
