%% caricamento
% Ogni blocco che comincia con %% è una cella: Ctrl+Shift+X ne manda una
% alla sessione già aperta, senza far ripartire tutto da capo.
t = linspace(0, 4*pi, 400);
ampiezza = 2.5;

%% segnale
segnale = ampiezza * sin(t) .* exp(-t/10);
rumore  = 0.15 * randn(size(t));
misura  = segnale + rumore;

%% grafico
figure(1);
plot(t, misura, 'Color', [0.7 0.7 0.9]);
hold on;
plot(t, segnale, 'LineWidth', 2);
hold off;
grid on;
xlabel('tempo [s]');
ylabel('ampiezza');
title('segnale smorzato e misura');
