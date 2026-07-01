<script setup lang="ts">
import { highlightText, type ShjLanguage } from "@speed-highlight/core";

const props = defineProps<{
  content: string;
  language: ShjLanguage;
  maxHeightClass?: string;
}>();

const colorMode = useColorMode();
const highlighted = ref("");

watch(
  () => [props.content, props.language] as const,
  async ([content, language]) => {
    const raw = content.trim();
    if (!raw) {
      highlighted.value = "";
      return;
    }
    try {
      highlighted.value = await highlightText(raw, language, false);
    } catch {
      highlighted.value = "";
    }
  },
  { immediate: true },
);
</script>

<template>
  <pre
    v-if="highlighted"
    dir="ltr"
    lang="en"
    class="code-highlight text-xs overflow-auto rounded-md border border-default bg-muted/30 p-3 text-left"
    :class="[
      maxHeightClass ?? 'max-h-[60vh]',
      colorMode.value === 'dark' ? 'code-highlight-dark' : 'code-highlight-light',
    ]"
  >
    <code
      dir="ltr"
      class="block font-mono whitespace-pre-wrap break-words"
      v-html="highlighted"
    />
  </pre>
</template>

<style scoped>
.code-highlight-light :deep(.shj-syn-kwd),
.code-highlight-light :deep(.shj-syn-err) {
  color: #e16;
}

.code-highlight-light :deep(.shj-syn-num),
.code-highlight-light :deep(.shj-syn-class) {
  color: #f60;
}

.code-highlight-light :deep(.shj-syn-cmnt) {
  color: #999;
}

.code-highlight-light :deep(.shj-syn-insert),
.code-highlight-light :deep(.shj-syn-str) {
  color: #3a7;
}

.code-highlight-light :deep(.shj-syn-bool) {
  color: #3bf;
}

.code-highlight-light :deep(.shj-syn-type),
.code-highlight-light :deep(.shj-syn-oper) {
  color: #5af;
}

.code-highlight-light :deep(.shj-syn-section),
.code-highlight-light :deep(.shj-syn-func) {
  color: #84f;
}

.code-highlight-light :deep(.shj-syn-deleted),
.code-highlight-light :deep(.shj-syn-var) {
  color: #c44;
}

.code-highlight-dark :deep(.shj-syn-insert),
.code-highlight-dark :deep(.shj-syn-str) {
  color: #98c379;
}

.code-highlight-dark :deep(.shj-syn-deleted),
.code-highlight-dark :deep(.shj-syn-err),
.code-highlight-dark :deep(.shj-syn-kwd) {
  color: #ff7b72;
}

.code-highlight-dark :deep(.shj-syn-class) {
  color: #ffa657;
}

.code-highlight-dark :deep(.shj-syn-cmnt) {
  color: #8b949e;
}

.code-highlight-dark :deep(.shj-syn-type),
.code-highlight-dark :deep(.shj-syn-oper),
.code-highlight-dark :deep(.shj-syn-num),
.code-highlight-dark :deep(.shj-syn-section),
.code-highlight-dark :deep(.shj-syn-var),
.code-highlight-dark :deep(.shj-syn-bool) {
  color: #79c0ff;
}

.code-highlight-dark :deep(.shj-syn-func) {
  color: #d2a8ff;
}
</style>
