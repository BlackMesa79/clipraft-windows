<script setup lang="ts">
import UiIcon from './UiIcon.vue';

withDefaults(
  defineProps<{
    modelValue: boolean;
    label: string;
    fluent: boolean;
    fluentLabel?: string;
    description?: string;
    descriptionId?: string;
    icon?: string;
    disabled?: boolean;
  }>(),
  { disabled: false },
);

const emit = defineEmits<{ 'update:modelValue': [value: boolean] }>();

function update(event: Event): void {
  const input = event.currentTarget;
  if (input instanceof HTMLInputElement) emit('update:modelValue', input.checked);
}
</script>

<template>
  <label class="check">
    <input
      type="checkbox"
      :checked="modelValue"
      :role="fluent ? 'switch' : undefined"
      :aria-describedby="fluent ? descriptionId : undefined"
      :disabled="disabled"
      @change="update"
    />
    <span v-if="fluent" class="appearance-label">
      <UiIcon v-if="icon" class="appearance-icon-slot" :name="icon" />
      <span class="appearance-label-copy">
        {{ fluentLabel ?? label }}
        <small v-if="description" :id="descriptionId">{{ description }}</small>
      </span>
    </span>
    <span v-else>{{ label }}</span>
  </label>
</template>
