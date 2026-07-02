RSpec.describe Cart do
  it "counts added items" do
    cart = described_class.new
    cart.add(:book)
    expect(cart.count).to eq(1)
  end
end

