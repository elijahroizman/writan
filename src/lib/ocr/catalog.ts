export const TESSDATA_COMMIT = '87416418657359cb625c412a48b6e1d6d41c29bd'
export const TESSDATA_BASE_URL = `https://raw.githubusercontent.com/tesseract-ocr/tessdata_fast/${TESSDATA_COMMIT}/`

// Mirrored by src-tauri/src/ocr.rs, which verifies native downloads independently.
export const OCR_PACK_CATALOG = [
  { code: 'eng', name: 'English', nativeName: 'English', primary: true, rtl: false, size: 4113088, sha256: '7d4322bd2a7749724879683fc3912cb542f19906c83bcc1a52132556427170b2' },
  { code: 'heb', name: 'Hebrew', nativeName: 'עברית', primary: true, rtl: true, size: 961404, sha256: '11f9e43ab227f786352a50f75c94c2e9906f1baba86d93276da19da7ce0904db' },
  { code: 'ara', name: 'Arabic', nativeName: 'العربية', primary: true, rtl: true, size: 1432056, sha256: 'e3206d3dc87fd50c24a0fb9f01838615911d25168f4e64415244b67d2bb3e729' },
  { code: 'fra', name: 'French', nativeName: 'Français', primary: false, rtl: false, size: 1130365, sha256: 'ced037562e8c80c13122dece28dd477d399af80911a28791a66a63ac1e3445ca' },
  { code: 'deu', name: 'German', nativeName: 'Deutsch', primary: false, rtl: false, size: 1525436, sha256: '19d219bbb6672c869d20a9636c6816a81eb9a71796cb93ebe0cb1530e2cdb22d' },
  { code: 'spa', name: 'Spanish', nativeName: 'Español', primary: false, rtl: false, size: 2294433, sha256: '6f2e04d02774a18f01bed44b1111f2cd7f3ba7ac9dc4373cd3f898a40ea6b464' },
  { code: 'ita', name: 'Italian', nativeName: 'Italiano', primary: false, rtl: false, size: 2701314, sha256: 'b8f89e1e785118dac4d51ae042c029a64edb5c3ee42ef73027a6d412748d8827' },
  { code: 'por', name: 'Portuguese', nativeName: 'Português', primary: false, rtl: false, size: 1982756, sha256: 'c4932b937207a9514b7514d518b931a99938c02a28a5a5a553f8599ed58b7deb' },
  { code: 'rus', name: 'Russian', nativeName: 'Русский', primary: false, rtl: false, size: 3861738, sha256: 'e16e5e036cce1d9ec2b00063cf8b54472625b9e14d893a169e2b0dedeb4df225' },
  { code: 'chi_sim', name: 'Chinese Simplified', nativeName: '简体中文', primary: false, rtl: false, size: 2469156, sha256: 'a5fcb6f0db1e1d6d8522f39db4e848f05984669172e584e8d76b6b3141e1f730' },
  { code: 'jpn', name: 'Japanese', nativeName: '日本語', primary: false, rtl: false, size: 2471260, sha256: '1f5de9236d2e85f5fdf4b3c500f2d4926f8d9449f28f5394472d9e8d83b91b4d' },
  { code: 'kor', name: 'Korean', nativeName: '한국어', primary: false, rtl: false, size: 1677415, sha256: '6b85e11d9bbf07863b97b3523b1b112844c43e713df8b66418a081fd1060b3b2' },
] as const

export type OcrPackMeta = (typeof OCR_PACK_CATALOG)[number]
export type OcrPackCode = OcrPackMeta['code']

export function getPackMeta(code: string): OcrPackMeta | undefined {
  return OCR_PACK_CATALOG.find((pack) => pack.code === code)
}

export function isRtlPack(code: string): boolean {
  return getPackMeta(code)?.rtl ?? false
}

export function formatBytes(bytes: number): string {
  if (bytes >= 1024 ** 3) return `${(bytes / 1024 ** 3).toFixed(1)} GB`
  if (bytes >= 1024 ** 2) return `${(bytes / 1024 ** 2).toFixed(1)} MB`
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${bytes} B`
}
