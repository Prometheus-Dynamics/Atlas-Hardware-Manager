import { writable } from "svelte/store";

export interface PageAction {
  label: string;
  className?: string;
}

export interface PageHeaderState {
  eyebrow?: string | null;
  title: string;
  subtitle?: string | null;
  actions?: PageAction[];
}

const defaultState: PageHeaderState = {
  eyebrow: null,
  title: "Atlas Hardware Manager",
  subtitle: "Fleet control workspace",
  actions: [],
};

const { subscribe, set, update } = writable<PageHeaderState>(defaultState);

export const pageHeader = {
  subscribe,
};

export const setPageHeader = (state: PageHeaderState) => {
  set({
    ...defaultState,
    ...state,
    actions: state.actions ?? [],
  });
};

export const updatePageHeader = (updater: (current: PageHeaderState) => PageHeaderState) =>
  update((current) => {
    const result = updater(current);
    return {
      ...defaultState,
      ...result,
      actions: result.actions ?? [],
    };
  });

export const resetPageHeader = () => set(defaultState);
