import { create } from "zustand";

/** Whether a controller is driving the app and whether the big-format layout is on. In a
 * store because the overlay, sidebar and grid all read it. */
interface CouchState {
  /** A controller is attached and being read. */
  connected: boolean;
  /** The pad's name, for saying which one. */
  name: string | null;
  /** The large-format layout is active. */
  couch: boolean;
  /** The controls overlay is showing. */
  helpOpen: boolean;
  setConnected: (connected: boolean, name: string | null) => void;
  setCouch: (couch: boolean) => void;
  toggleHelp: () => void;
  closeHelp: () => void;
}

export const useCouch = create<CouchState>((set) => ({
  connected: false,
  name: null,
  couch: false,
  helpOpen: false,
  setConnected: (connected, name) =>
    set((state) => ({
      connected,
      name,
      // Unplugging a pad returns the app to its normal size; leaving it in couch mode
      // would strand a mouse user in a layout built to be read from a sofa.
      couch: connected ? state.couch : false,
      helpOpen: connected ? state.helpOpen : false,
    })),
  setCouch: (couch) => set({ couch }),
  toggleHelp: () => set((state) => ({ helpOpen: !state.helpOpen })),
  closeHelp: () => set({ helpOpen: false }),
}));
