import UIKit

/// 键盘上方的一排特殊键（终端与远程桌面共用）：横向滚动，修饰键是粘滞开关。
final class KeyBarView: UIInputView {
    enum Item {
        case key(id: String, title: String?, symbol: String?)
        case toggle(id: String, title: String)
        case action(id: String, symbol: String)
        case separator
    }

    var onKey: ((String) -> Void)?
    var onToggle: ((String, Bool) -> Void)?
    private(set) var toggles: [String: Bool] = [:]
    private var toggleButtons: [String: UIButton] = [:]
    private let stack = UIStackView()
    private let scroll = UIScrollView()
    var tint: UIColor = .systemBlue { didSet { refreshToggles() } }

    init(items: [Item], height: CGFloat = 46) {
        super.init(frame: CGRect(x: 0, y: 0, width: 320, height: height), inputViewStyle: .keyboard)
        allowsSelfSizing = true
        autoresizingMask = .flexibleWidth
        scroll.showsHorizontalScrollIndicator = false
        scroll.translatesAutoresizingMaskIntoConstraints = false
        addSubview(scroll)
        stack.axis = .horizontal
        stack.spacing = 6
        stack.alignment = .center
        stack.translatesAutoresizingMaskIntoConstraints = false
        scroll.addSubview(stack)
        NSLayoutConstraint.activate([
            scroll.leadingAnchor.constraint(equalTo: leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: trailingAnchor),
            scroll.topAnchor.constraint(equalTo: topAnchor),
            scroll.bottomAnchor.constraint(equalTo: bottomAnchor),
            heightAnchor.constraint(equalToConstant: height),
            stack.leadingAnchor.constraint(equalTo: scroll.contentLayoutGuide.leadingAnchor, constant: 8),
            stack.trailingAnchor.constraint(equalTo: scroll.contentLayoutGuide.trailingAnchor, constant: -8),
            stack.centerYAnchor.constraint(equalTo: scroll.centerYAnchor),
            stack.heightAnchor.constraint(equalToConstant: height - 10),
        ])
        for item in items { stack.addArrangedSubview(makeView(item)) }
    }

    required init?(coder: NSCoder) { fatalError() }

    override var intrinsicContentSize: CGSize { CGSize(width: UIView.noIntrinsicMetric, height: bounds.height) }

    private func makeView(_ item: Item) -> UIView {
        switch item {
        case .separator:
            let v = UIView()
            v.backgroundColor = UIColor.separator
            v.translatesAutoresizingMaskIntoConstraints = false
            v.widthAnchor.constraint(equalToConstant: 1).isActive = true
            v.heightAnchor.constraint(equalToConstant: 22).isActive = true
            return v
        case .key(let id, let title, let symbol):
            return button(title: title, symbol: symbol) { [weak self] in self?.onKey?(id) }
        case .action(let id, let symbol):
            let b = button(title: nil, symbol: symbol) { [weak self] in self?.onKey?(id) }
            b.configuration?.baseBackgroundColor = .clear
            return b
        case .toggle(let id, let title):
            let b = button(title: title, symbol: nil) { [weak self] in
                guard let self else { return }
                let on = !(self.toggles[id] ?? false)
                self.toggles[id] = on
                self.refreshToggles()
                self.onToggle?(id, on)
            }
            toggleButtons[id] = b
            return b
        }
    }

    private func button(title: String?, symbol: String?, action: @escaping () -> Void) -> UIButton {
        var c = UIButton.Configuration.filled()
        c.baseBackgroundColor = UIColor.secondarySystemBackground.withAlphaComponent(0.9)
        c.baseForegroundColor = .label
        c.cornerStyle = .medium
        c.contentInsets = NSDirectionalEdgeInsets(top: 6, leading: 10, bottom: 6, trailing: 10)
        if let title {
            var a = AttributedString(title)
            a.font = UIFont.monospacedSystemFont(ofSize: 14, weight: .medium)
            c.attributedTitle = a
        }
        if let symbol {
            c.image = UIImage(systemName: symbol, withConfiguration: UIImage.SymbolConfiguration(pointSize: 14, weight: .medium))
        }
        let b = UIButton(configuration: c, primaryAction: UIAction { _ in
            UIDevice.current.playInputClick()
            action()
        })
        b.translatesAutoresizingMaskIntoConstraints = false
        b.widthAnchor.constraint(greaterThanOrEqualToConstant: 38).isActive = true
        return b
    }

    /// 修饰键用过一次后自动松开
    func setToggle(_ id: String, _ on: Bool) {
        toggles[id] = on
        refreshToggles()
    }

    func clearToggles() {
        for k in toggles.keys { toggles[k] = false }
        refreshToggles()
    }

    private func refreshToggles() {
        for (id, b) in toggleButtons {
            let on = toggles[id] ?? false
            b.configuration?.baseBackgroundColor = on ? tint : UIColor.secondarySystemBackground.withAlphaComponent(0.9)
            b.configuration?.baseForegroundColor = on ? .white : .label
        }
    }
}

extension KeyBarView: UIInputViewAudioFeedback {
    var enableInputClicksWhenVisible: Bool { true }
}
