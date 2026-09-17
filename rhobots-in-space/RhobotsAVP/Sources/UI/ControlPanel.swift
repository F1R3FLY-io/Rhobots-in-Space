//  ControlPanel.swift
//  Controls that tune the environment rather than edit the program.
//
//  Five groups, top to bottom, in the order the spec gives them: execution,
//  concurrency, playback, motion, rendering. Changing any control never
//  changes the term being run. The panel persists its settings between
//  sessions, and sits at or above eye level — eye tracking was unreliable in
//  the lower field during the F1R3Skein bring-up, and that finding is carried
//  over here rather than rediscovered.

import SwiftUI

struct ControlPanel: View {

    @Bindable var world: World
    @Environment(\.openImmersiveSpace) private var openSpace
    @Environment(\.dismissImmersiveSpace) private var dismissSpace
    @State private var spaceOpen = false

    private var panel: Panel { world.panel }

    var body: some View {
        NavigationStack {
            Form {
                connection
                term
                execution
                concurrency
                playback
                motion
                rendering
            }
            .navigationTitle("Rhobots")
        }
        .frame(minWidth: 460, minHeight: 640)
        .onAppear { world.connect() }
    }

    // MARK: Connection

    private var connection: some View {
        Section("Executive") {
            HStack {
                TextField("host", text: $world.panel.host)
                    .textFieldStyle(.roundedBorder)
                TextField("port", value: $world.panel.port, format: .number)
                    .textFieldStyle(.roundedBorder)
                    .frame(width: 90)
                Button("Connect") { world.connect() }
            }
            HStack(spacing: 8) {
                Circle()
                    .fill(statusColor)
                    .frame(width: 10, height: 10)
                Text(statusText).foregroundStyle(.secondary)
            }
            if let notice = world.notice {
                Text(notice).font(.footnote).foregroundStyle(.secondary)
            }
        }
    }

    private var statusColor: Color {
        switch world.connection {
        case .connected: return .green
        case .connecting: return .orange
        case .failed: return .red
        case .idle: return .gray
        }
    }

    private var statusText: String {
        switch world.connection {
        case .idle: return "not connected"
        case .connecting: return "connecting — the world on the Mac is kept while this reconnects"
        case .connected(let s, let resumed): return resumed ? "resumed session \(s)" : "session \(s)"
        case .failed(let m): return m
        }
    }

    // MARK: The term

    private var term: some View {
        Section("Term") {
            TextEditor(text: $world.source)
                .font(.system(.body, design: .monospaced))
                .frame(minHeight: 140)
            HStack {
                Button("Deploy") { world.deploy() }
                    .buttonStyle(.borderedProminent)
                Button("Reset") { world.reset() }
                Spacer()
                if !spaceOpen {
                    Button("Enter the space") {
                        Task {
                            _ = await openSpace(id: "rhobots")
                            spaceOpen = true
                        }
                    }
                } else {
                    Button("Leave") {
                        Task {
                            await dismissSpace()
                            spaceOpen = false
                        }
                    }
                }
            }
            ForEach(world.diagnostics.indices, id: \.self) { i in
                let d = world.diagnostics[i]
                Text("\(d.lo)–\(d.hi)  \(d.code): \(d.message)")
                    .font(.footnote.monospaced())
                    .foregroundStyle(.red)
            }
        }
    }

    // MARK: Execution (§9.2)

    private var execution: some View {
        Section("Execution") {
            HStack(spacing: 16) {
                Button(panel.running ? "Pause" : "Run") { world.playPause() }
                    .buttonStyle(.borderedProminent)
                Button("Step round") { world.stepOnce() }
                Spacer()
            }
            Picker("Pacing", selection: Binding(
                get: { panel.lockstep },
                set: { panel.lockstep = $0; world.pushPanel() })) {
                Text("Lockstep").tag(true)
                Text("Free-running").tag(false)
            }
            .pickerStyle(.segmented)
            Text(panel.lockstep
                 ? "A round of at most \(panel.cores) commits is admitted, then the next waits until the scene has finished animating this one."
                 : "The evaluator commits without waiting. The buffer applies back-pressure, so it never runs further ahead of the scene than its bound, and every commit is animated.")
                .font(.footnote)
                .foregroundStyle(.secondary)
        }
    }

    // MARK: Concurrency (§9.3)

    private var concurrency: some View {
        Section("Concurrency") {
            Stepper(value: Binding(
                get: { panel.cores },
                set: { panel.cores = $0; world.pushPanel() }),
                    in: 1...max(panel.coresMax, 1)) {
                Text("Round width  \(panel.cores) of \(panel.coresMax)")
            }
            if let s = world.stats {
                LabeledContent("Throughput", value: "\(s.commitsPerSec) commits/s")
                LabeledContent("Commits", value: "\(s.comms)")
            }
            Text("The principal tool for getting a feel for races and for scaling. Communications contending for one channel serialise on that channel whatever the width is, and the readout will show the plateau — which is itself the lesson.")
                .font(.footnote)
                .foregroundStyle(.secondary)
        }
    }

    // MARK: Playback (§9.2)

    private var playback: some View {
        Section("Playback") {
            slider("Communication duration", value: Binding(
                get: { panel.commDuration },
                set: { panel.commDuration = $0; panel.save() }),
                   range: 0.2...4, unit: "s")
            if !panel.lockstep {
                Stepper(value: Binding(
                    get: { panel.bufferBound },
                    set: { panel.bufferBound = $0; world.pushPanel() }),
                        in: 16...1024, step: 16) {
                    Text("Buffer  \(panel.bufferBound) rounds")
                }
            }
            if let s = world.stats {
                ProgressView(value: Double(s.inFlight), total: Double(max(s.buffer, 1))) {
                    Text("Buffer fill  \(s.inFlight) of \(s.buffer)")
                }
                Label(s.admissionOpen ? "Admission open" : "Admission paused by back-pressure",
                      systemImage: s.admissionOpen ? "arrow.right.circle" : "pause.circle")
                    .foregroundStyle(s.admissionOpen ? .secondary : .orange)
                    .font(.footnote)
            }
        }
    }

    // MARK: Motion (§7.1, §7.2)

    private var motion: some View {
        Section("Motion") {
            slider("Wander speed", value: Binding(
                get: { panel.wanderSpeed }, set: { panel.wanderSpeed = $0; panel.save() }),
                   range: 0...0.3, unit: "m/s")
            Text(panel.wanderSpeed == 0
                 ? "The plane is still."
                 : "Position on a plane carries no meaning, so robots drift. Two matching fingerprints may come close without anything happening.")
                .font(.footnote).foregroundStyle(.secondary)
            slider("Approach speed", value: Binding(
                get: { panel.approachSpeed }, set: { panel.approachSpeed = $0; panel.save() }),
                   range: 0.1...1.5, unit: "m/s")
            Picker("Cables", selection: Binding(
                get: { panel.breakFreely },
                set: { panel.breakFreely = $0; panel.save() })) {
                Text("Avoid, then break").tag(false)
                Text("Break freely").tag(true)
            }
            .pickerStyle(.segmented)
        }
    }

    // MARK: Rendering (§3)

    private var rendering: some View {
        Section("Rendering") {
            slider("Shrink factor σ", value: $world.panel.shrink, range: 0.15...0.6, unit: "")
            slider("Legibility threshold", value: $world.panel.legibility,
                   range: 0.02...0.2, unit: "")
            slider("Plane spacing", value: $world.panel.planeSpacing,
                   range: 0.15...1.0, unit: "m")
            Text("Quotation levels drawn: \(panel.quoteDepth). Past that a subterm is an elision glyph — which keeps its fingerprint, so an elided name stays comparable.")
                .font(.footnote).foregroundStyle(.secondary)
            slider("Inert fade delay", value: $world.panel.inertFade, range: 0...10, unit: "s")
        }
    }

    // MARK: Parts

    private func slider(_ title: String, value: Binding<Double>,
                        range: ClosedRange<Double>, unit: String) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack {
                Text(title)
                Spacer()
                Text(String(format: "%.2f", value.wrappedValue) + unit)
                    .foregroundStyle(.secondary)
                    .monospacedDigit()
            }
            Slider(value: value, in: range)
        }
    }
}
