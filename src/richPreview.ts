export type RichBlock =
  { kind: 'text'; text: string } | { kind: 'image'; src: string | null; message: string | null };
export interface RichPreview {
  blocks: RichBlock[];
  truncated: boolean;
  linked_images: boolean;
}
