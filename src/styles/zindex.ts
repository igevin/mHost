// Z-index layer constants to avoid magic number conflicts
export const Z_INDEX = {
  // ImportDialog overlay/dialog
  DIALOG_OVERLAY: 100,
  DIALOG_CONTENT: 110,
  // Create Profile dialogs (inside Layout, ProfileView, ManagementDrawer)
  CREATE_OVERLAY: 200,
  CREATE_CONTENT: 210,
  // ManagementDrawer (needs to be above create dialogs in sidebar)
  DRAWER_OVERLAY: 300,
  DRAWER_CONTENT: 310,
  // Create dialog inside drawer (needs to be above drawer)
  DRAWER_CREATE_OVERLAY: 400,
  DRAWER_CREATE_CONTENT: 410,
} as const;
