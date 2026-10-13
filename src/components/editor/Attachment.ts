/**
 * Tiptap 自定义节点：媒体附件块。
 * - 文档中以 <figure data-attach-id="N"></figure> 存储（block + atom + selectable）
 * - 编辑态由 MediaBlock.vue（VueNodeViewRenderer）渲染为媒体元素 + 文件名 + 拖拽手柄
 * - 原子节点：内部不可编辑，退格/Delete 键整体删除，点选后可 Ctrl+C 复制
 */
import { Node, mergeAttributes } from "@tiptap/core";
import { VueNodeViewRenderer } from "@tiptap/vue-3";
import MediaBlock from "./MediaBlock.vue";

export const Attachment = Node.create({
  name: "attachment",
  group: "block",
  atom: true,
  selectable: true,
  draggable: false,

  addAttributes() {
    return {
      id: {
        default: null,
        parseHTML: (el: HTMLElement) => {
          const v = el.getAttribute("data-attach-id");
          return v != null && v !== "" ? Number(v) : null;
        },
        renderHTML: (attrs: Record<string, unknown>) => ({
          "data-attach-id": attrs.id ?? "",
        }),
      },
    };
  },

  parseHTML() {
    return [{ tag: "figure[data-attach-id]" }];
  },

  renderHTML({ HTMLAttributes }) {
    // 存储形态保持简单；复制粘贴到外部应用时以 figure 呈现。
    // 注意：atom 叶子节点 spec 不允许内容洞（0），figcaption 只能静态输出
    return ["figure", mergeAttributes(HTMLAttributes), ["figcaption"]];
  },

  addNodeView() {
    return VueNodeViewRenderer(MediaBlock);
  },
});

export default Attachment;
