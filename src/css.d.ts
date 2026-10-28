declare module "*.module.css" {
  const classes: Record<string, string>;
  export default classes;
}

declare module "*.css";

declare module "*?raw" {
  const content: string;
  export default content;
}

declare const __APP_VERSION__: string;
