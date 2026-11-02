// Watch parent module + local frame-camera so native/JS edits reload.
const { getDefaultConfig } = require("expo/metro-config");
const path = require("path");

const projectRoot = __dirname;
const moduleRoot = path.resolve(projectRoot, "..");
const frameCameraRoot = path.resolve(projectRoot, "modules/frame-camera");

const config = getDefaultConfig(projectRoot);

config.watchFolders = [moduleRoot, frameCameraRoot];
config.resolver.nodeModulesPaths = [
  path.resolve(projectRoot, "node_modules"),
  path.resolve(moduleRoot, "node_modules"),
];
// Keep Metro's hierarchical lookup enabled: npm may install version-specific
// dependencies under expo/node_modules (for example @expo/log-box). The
// explicit paths above remain fallbacks for the linked local modules.

// Still allow .luma assets if fixtures are used later.
if (!config.resolver.assetExts.includes("luma")) {
  config.resolver.assetExts.push("luma");
}

module.exports = config;
