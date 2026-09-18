//  ImmersiveView.swift
//  The scene itself.
//
//  Hand tracking is available only in an `ImmersiveSpace`, so this is where
//  the robots live. The control panel is a separate window placed beside the
//  live plane rather than in front of it, because anything in front of z₀
//  would be read as standing on it.

import RealityKit
import SwiftUI

struct ImmersiveView: View {

    let world: World
    @State private var director: SceneDirector?

    var body: some View {
        RealityView { content in
            let d = SceneDirector(world: world)
            director = d
            content.add(d.root)
            // Draw whatever has already arrived, so a person entering the
            // space mid-run sees the world rather than an empty plane.
            d.apply(world.scene, unshrinkNew: false)
        } update: { _ in
            // Scene changes are applied by the director when a round finishes
            // or an unprompted scene arrives, not on every SwiftUI update: a
            // rebuild here would fight the animation.
        }
        .task {
            // The frame loop. Wandering, cable lag and the approach all
            // advance here; rounds are pulled from the buffer here too.
            while !Task.isCancelled {
                director?.update()
                try? await Task.sleep(nanoseconds: 16_000_000)
            }
        }
        .onChange(of: world.scene.tick) {
            // An unprompted scene — a deploy, a reset, a reconnect — is
            // applied directly. A round's scene goes through the animation.
            if world.playing == nil {
                director?.apply(world.scene, unshrinkNew: false)
            }
        }
        .onChange(of: world.panel.shrink) { relayout() }
        .onChange(of: world.panel.legibility) { relayout() }
        .onChange(of: world.panel.planeSpacing) { relayout() }
        .gesture(
            // A robot the programmer is grabbing holds still. This is the
            // construction gesture's foundation; the rest of the editing
            // vocabulary is to be settled on device.
            DragGesture()
                .targetedToAnyEntity()
                .onChanged { value in
                    guard let id = cardID(of: value.entity) else { return }
                    director?.hold(id, true)
                    value.entity.position = value.convert(value.location3D,
                                                          from: .local,
                                                          to: value.entity.parent!)
                }
                .onEnded { value in
                    if let id = cardID(of: value.entity) { director?.hold(id, false) }
                }
        )
    }

    /// Sliders that affect layout re-lay the scene smoothly while it runs,
    /// since they change only the rendering function and never the term.
    private func relayout() {
        world.client.setTreeDepth(world.panel.quoteDepth)
        world.panel.save()
        if world.playing == nil {
            director?.apply(world.scene, unshrinkNew: false)
        }
    }

    /// Walk up to the card an entity belongs to. Identity comes from the
    /// executive, never from where something happens to sit.
    private func cardID(of entity: Entity) -> String? {
        var e: Entity? = entity
        while let current = e {
            if current.name.hasPrefix("card:") {
                return String(current.name.dropFirst("card:".count))
            }
            e = current.parent
        }
        return nil
    }
}
