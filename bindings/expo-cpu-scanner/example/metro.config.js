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
config.resolver.disableHierarchicalLookup = true;

// Still allow .luma assets if fixtures are used later.
if (!config.resolver.assetExts.includes("luma")) {
  config.resolver.assetExts.push("luma");
}

module.exports = config;
