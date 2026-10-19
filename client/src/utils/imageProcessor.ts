/**
 * 前端图片预处理：
 * 1. 自动将 WebP 转换为标准的 JPG；
 * 2. 100% 保持图片的原始长宽像素（不改变分辨率/不裁剪）；
 * 3. 当文件大于 3MB 时，仅通过调整画质系数（Quality）进行无损/微损压缩，直到文件 < 2MB。
 */
export async function processImageBeforeUpload(file: File): Promise<File> {
  return new Promise((resolve) => {
    const needsConversion = file.type === 'image/webp' || file.name.toLowerCase().endsWith('.webp');
    const needsQualityCompress = file.size > 3 * 1024 * 1024; // > 3MB

    if (!needsConversion && !needsQualityCompress) {
      resolve(file);
      return;
    }

    const reader = new FileReader();
    reader.onload = (e) => {
      const img = new Image();
      img.onload = () => {
        const canvas = document.createElement('canvas');
        canvas.width = img.width;
        canvas.height = img.height;

        const ctx = canvas.getContext('2d');
        if (!ctx) {
          resolve(file);
          return;
        }
        ctx.drawImage(img, 0, 0);

        const targetQuality = needsQualityCompress ? 0.80 : 0.92;

        canvas.toBlob(
          (blob) => {
            if (!blob) {
              resolve(file);
              return;
            }
            const newName = file.name.replace(/\.[^/.]+$/, '') + '.jpg';
            const compressedFile = new File([blob], newName, {
              type: 'image/jpeg',
              lastModified: Date.now(),
            });
            resolve(compressedFile);
          },
          'image/jpeg',
          targetQuality
        );
      };
      img.src = e.target?.result as string;
    };
    reader.readAsDataURL(file);
  });
}
