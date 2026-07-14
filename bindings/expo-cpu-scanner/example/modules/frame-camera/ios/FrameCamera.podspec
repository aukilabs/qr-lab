Pod::Spec.new do |s|
  s.name           = 'FrameCamera'
  s.version        = '0.1.0'
  s.summary        = 'Real-time camera Y-plane frames for the qrk example app'
  s.description    = 'AVFoundation preview + sample buffer frames for feeding native scanners'
  s.author         = 'Auki Labs'
  s.homepage       = 'https://aukilabs.com'
  s.license        = 'MIT'
  s.platforms      = { :ios => '15.1' }
  s.source         = { git: '' }
  s.static_framework = true

  s.dependency 'ExpoModulesCore'

  s.pod_target_xcconfig = {
    'DEFINES_MODULE' => 'YES',
  }

  s.source_files = '**/*.{h,m,mm,swift}'
  s.frameworks = 'AVFoundation', 'UIKit', 'CoreVideo', 'CoreMedia'
  # FrameCameraSession.swift + FrameCameraView.swift + FrameCameraModule.swift
end
