  // The acoustic calibration plugin ships with the app (plugins/calibration.wasm,
  // gitignored build artifact). Its 删除 button is hidden: removing the file
  // silently kills the whole feature and there is no in-app way to get it back.
  const isCalibrationPlugin = (plugin: PluginInfo) => {
    const name = plugin.manifest.name.toLowerCase()
    return name === 'calibration' || name === 'acoustic calibration'
  }
