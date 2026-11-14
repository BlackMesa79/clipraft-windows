import { ref, type Ref } from 'vue';

export function createDetailImage(): {
  imageSource: Ref<string>;
  replaceImage: (image?: number[]) => void;
} {
  const imageSource = ref('');
  function replaceImage(image?: number[]): void {
    if (imageSource.value) URL.revokeObjectURL(imageSource.value);
    imageSource.value = image ? URL.createObjectURL(new Blob([new Uint8Array(image)])) : '';
  }
  return { imageSource, replaceImage };
}
