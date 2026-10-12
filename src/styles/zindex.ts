// Z-index layer constants to avoid magic number conflicts
export const Z_INDEX = {
  // ImportDialog overlay/dialog
  DIALOG_OVERLAY: 100,
  DIALOG_CONTENT: 110,
  // Create Profile dialogs (inside Sidebar, HostsProfileView)
  CREATE_OVERLAY: 200,
  CREATE_CONTENT: 210,
} as const;
