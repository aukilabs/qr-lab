Pod::Spec.new do |s|
  s.name           = 'ExpoCpuScanner'
  s.version        = '0.1.0'
  s.summary        = 'CPU QR scanner (qrk) for Expo'
  s.description    = 'Expo module wrapping the qrk Rust CPU QR scanner'
  s.author         = 'Auki Labs'
  s.homepage       = 'https://aukilabs.com'
  s.license        = 'MIT'
  s.platforms      = {
    :ios => '15.1',
    :tvos => '15.1'
  }
  s.source         = { git: '' }
  s.static_framework = true

  s.dependency 'ExpoModulesCore'

  # force_load per-SDK so device and simulator each pull their xcframework slice
  # (both slices are named libqrk_ffi.a — required by CocoaPods vendored_frameworks).
  s.pod_target_xcconfig = {
    'DEFINES_MODULE' => 'YES',
    'OTHER_LDFLAGS[sdk=iphoneos*]' =>
      '$(inherited) -ObjC -force_load "$(PODS_TARGET_SRCROOT)/Qrk.xcframework/ios-arm64/libqrk_ffi.a"',
    'OTHER_LDFLAGS[sdk=iphonesimulator*]' =>
      '$(inherited) -ObjC -force_load "$(PODS_TARGET_SRCROOT)/Qrk.xcframework/ios-arm64_x86_64-simulator/libqrk_ffi.a"',
  }

  s.source_files = 'ExpoCpuScannerModule.swift', 'QrkBridge.swift'
  s.vendored_frameworks = 'Qrk.xcframework'
  s.libraries = 'c++'
end
