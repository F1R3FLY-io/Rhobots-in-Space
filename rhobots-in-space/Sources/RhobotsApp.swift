//  RhobotsApp.swift
//  Programming the pure rho calculus in immersive space.
//
//  Two scenes: the control panel, a plain window, and the rhobots themselves,
//  an ImmersiveSpace — hand tracking is available only there.

import SwiftUI

@main
struct RhobotsApp: App {

    @State private var world = World()

    var body: some Scene {
        WindowGroup(id: "panel") {
            ControlPanel(world: world)
        }
        .defaultSize(width: 480, height: 700)

        ImmersiveSpace(id: "rhobots") {
            ImmersiveView(world: world)
        }
        // Mixed, not full: the live plane stands in the room, and the
        // programmer can still see where they are standing.
        .immersionStyle(selection: .constant(.mixed), in: .mixed)
    }
}
