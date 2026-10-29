// The backdrop overlay page (FR-20, FR-46, spec 7.9). The window's
// background layer carries the solid colour; this page adds gradients,
// pictures and the wallpaper on top and fades them with blur/dim. The
// page reloads whenever settings change (backdrop::restyle), so it reads
// the current style once on load and listens for nothing.
import { getBackdropImage, getBackdropStyle } from "../shared/ipc";
import "./backdrop.css";

/** Magic-byte sniff for the blob type (mirrors the Rust reader, which
 * already rejected unsupported/empty files; Win11's TranscodedWallpaper
 * has no extension, so the content decides). */
function imageMime(bytes: ArrayBuffer): string | null {
  const view = new DataView(bytes);
  if (bytes.byteLength < 12) return null;
  const b = new Uint8Array(bytes, 0, 4);
  if (b[0] === 0x89 && b[1] === 0x50 && b[2] === 0x4e && b[3] === 0x47) {
    return "image/png";
  }
  if (b[0] === 0xff && b[1] === 0xd8 && b[2] === 0xff) return "image/jpeg";
  if (b[0] === 0x42 && b[1] === 0x4d) return "image/bmp";
  const riff = String.fromCharCode(b[0], b[1], b[2], b[3]);
  const webp = String.fromCharCode(
    view.getUint8(8),
    view.getUint8(9),
    view.getUint8(10),
    view.getUint8(11),
  );
  if (riff === "RIFF" && webp === "WEBP") return "image/webp";
  return null;
}

/** Read the image as a data URL (CSP img-src allows data:). */
function readAsDataURL(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.addEventListener("load", () => resolve(String(reader.result)));
    reader.addEventListener("error", () => reject(reader.error));
    reader.readAsDataURL(blob);
  });
}

async function render(): Promise<void> {
  const style = await getBackdropStyle();
  const image = document.getElementById(
    "backdrop-image",
  ) as HTMLImageElement | null;
  const dim = document.getElementById("backdrop-dim") as HTMLElement | null;

  if (style.style === "gradient" && style.gradient) {
    // The preset ids are validated in Rust (D-087); an unknown one simply
    // matches no class and the colour layer shows through.
    document.body.classList.add(`gradient-${style.gradient}`);
  }

  if (
    image &&
    dim &&
    (style.style === "picture" || style.style === "wallpaper")
  ) {
    const bytes = await getBackdropImage();
    const mime = bytes.byteLength > 0 ? imageMime(bytes) : null;
    if (mime) {
      const url = await readAsDataURL(new Blob([bytes], { type: mime }));
      image.src = url;
      image.hidden = false;
      if (style.blur > 0) {
        image.style.filter = `blur(${style.blur}px)`;
      }
      if (dim && style.dim > 0) {
        dim.hidden = false;
        dim.style.opacity = String(style.dim / 100);
      }
    }
    // No usable image: the colour layer stays the backdrop (spec 7.9
    // fallback), nothing to do here.
  }
}

void render();
